struct FindEntry {
    name: String,
    attrs: u32,
    size: u64,
}

enum VirtualOpen {
    Opened(u32),
    Failed(u32),
    Miss,
}

enum FileOpen {
    Opened(u32),
    Failed(u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DriveDevice {
    Fixed,
    Cdrom,
    Virtual,
}

impl DriveDevice {
    fn from_app_db(value: &str) -> Self {
        match value {
            "cdrom" => Self::Cdrom,
            "virtual" => Self::Virtual,
            _ => Self::Fixed,
        }
    }

    fn win32_drive_type(self, drive: char) -> u32 {
        match self {
            Self::Cdrom => 5,
            Self::Fixed | Self::Virtual => {
                if matches!(drive, 'A' | 'B') {
                    2
                } else {
                    3
                }
            }
        }
    }

    fn volume_name(self) -> &'static str {
        match self {
            Self::Cdrom => "WEMU_CD",
            Self::Fixed | Self::Virtual => "WEMU",
        }
    }
}

struct DriveTable {
    mounts: [Option<PathBuf>; 26],
    devices: [DriveDevice; 26],
    virtual_present: [bool; 26],
    aliases: [Option<String>; 26],
}

impl Default for DriveTable {
    fn default() -> Self {
        Self {
            mounts: std::array::from_fn(|_| None),
            devices: [DriveDevice::Fixed; 26],
            virtual_present: [false; 26],
            aliases: std::array::from_fn(|_| None),
        }
    }
}

impl DriveTable {
    fn set_mount(&mut self, idx: usize, path: PathBuf) {
        self.mounts[idx] = Some(path);
        self.aliases[idx] = None;
    }

    fn set_device(&mut self, idx: usize, device: DriveDevice) {
        self.devices[idx] = device;
    }

    fn set_alias(&mut self, idx: usize, target_key: String, device: DriveDevice) {
        self.aliases[idx] = Some(target_key);
        self.devices[idx] = device;
    }

    fn mounted_at(&self, idx: usize) -> bool {
        self.mounts[idx].is_some() || self.virtual_present[idx] || self.aliases[idx].is_some()
    }

    fn device_at(&self, idx: usize) -> DriveDevice {
        self.devices[idx]
    }

    fn alias_at(&self, idx: usize) -> Option<&str> {
        self.aliases[idx].as_deref()
    }

    fn mark_virtual_present(&mut self, idx: usize) {
        self.virtual_present[idx] = true;
    }

    fn host_root_at(&self, idx: usize) -> Option<&PathBuf> {
        self.mounts[idx].as_ref()
    }
}

#[derive(Clone, Copy)]
struct AsyncVfsEntry {
    size: u64,
    writable: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingVfsRequestKind {
    Read,
    Write,
}

struct PendingVfsRequest {
    id: u32,
    kind: PendingVfsRequestKind,
    path: String,
    offset: u64,
    len: u32,
    data: Vec<u8>,
}

pub(crate) struct CompletedVfsRequest {
    pub(crate) id: u32,
    pub(crate) status: u32,
    pub(crate) transferred: u32,
    pub(crate) data: Vec<u8>,
}

struct Vfs {
    drive_table: DriveTable,
    cwd_drive: char,
    cwd_path: String,
    enabled: bool,
    files: HashMap<String, Rc<RefCell<Vec<u8>>>>,
    async_entries: HashMap<String, AsyncVfsEntry>,
    async_writable: bool,
    next_request_id: u32,
    pending_request: Option<PendingVfsRequest>,
    completed_requests: Vec<CompletedVfsRequest>,
}

struct FindEntriesResult {
    dir_raw: String,
    pattern: String,
    host_dir: Option<PathBuf>,
    entries: Vec<FindEntry>,
}

impl Default for Vfs {
    fn default() -> Self {
        Self {
            drive_table: DriveTable::default(),
            cwd_drive: 'C',
            cwd_path: "\\".to_string(),
            enabled: false,
            files: HashMap::new(),
            async_entries: HashMap::new(),
            async_writable: false,
            next_request_id: 1,
            pending_request: None,
            completed_requests: Vec::new(),
        }
    }
}

impl Vfs {
    fn enable(&mut self) {
        self.enabled = true;
    }

    fn enable_async_writes(&mut self) {
        self.enabled = true;
        self.async_writable = true;
    }

    fn insert_file(&mut self, key: String, bytes: Vec<u8>) {
        self.files.insert(key, Rc::new(RefCell::new(bytes)));
    }

    fn insert_async_file(&mut self, key: String, size: u64, writable: bool) {
        self.async_entries
            .insert(key, AsyncVfsEntry { size, writable });
    }

    fn note_async_write(&mut self, key: &str, offset: u64, len: usize) {
        if let Some(entry) = self.async_entries.get_mut(key) {
            entry.size = entry.size.max(offset.saturating_add(len as u64));
            entry.writable = true;
        }
    }

    fn pending_request_id(&self) -> u32 {
        self.pending_request.as_ref().map_or(0, |req| req.id)
    }

    fn pending_request_kind(&self) -> u32 {
        match self.pending_request.as_ref().map(|req| req.kind) {
            Some(PendingVfsRequestKind::Read) => 1,
            Some(PendingVfsRequestKind::Write) => 2,
            None => 0,
        }
    }

    fn pending_request_path(&self) -> &[u8] {
        self.pending_request
            .as_ref()
            .map(|req| req.path.as_bytes())
            .unwrap_or(&[])
    }

    fn pending_request_offset(&self) -> u64 {
        self.pending_request.as_ref().map_or(0, |req| req.offset)
    }

    fn pending_request_len(&self) -> u32 {
        self.pending_request.as_ref().map_or(0, |req| req.len)
    }

    fn pending_request_data(&self) -> &[u8] {
        self.pending_request
            .as_ref()
            .map(|req| req.data.as_slice())
            .unwrap_or(&[])
    }

    fn complete_request(
        &mut self,
        request_id: u32,
        status: u32,
        transferred: u32,
        data: Vec<u8>,
    ) -> bool {
        let Some(req) = self.pending_request.take() else {
            return false;
        };
        if req.id != request_id {
            self.pending_request = Some(req);
            return false;
        }
        self.completed_requests.push(CompletedVfsRequest {
            id: request_id,
            status,
            transferred,
            data,
        });
        true
    }

    fn begin_read(&mut self, key: &str, offset: u64, len: u32) -> u32 {
        assert!(
            self.pending_request.is_none(),
            "new async VFS read while another request is pending"
        );
        let id = self.alloc_request_id();
        self.pending_request = Some(PendingVfsRequest {
            id,
            kind: PendingVfsRequestKind::Read,
            path: key.to_string(),
            offset,
            len,
            data: Vec::new(),
        });
        id
    }

    fn begin_write(&mut self, key: &str, offset: u64, data: Vec<u8>) -> u32 {
        assert!(
            self.pending_request.is_none(),
            "new async VFS write while another request is pending"
        );
        let id = self.alloc_request_id();
        self.pending_request = Some(PendingVfsRequest {
            id,
            kind: PendingVfsRequestKind::Write,
            path: key.to_string(),
            offset,
            len: data.len() as u32,
            data,
        });
        id
    }

    fn has_completed_request(&self, request_id: u32) -> bool {
        self.completed_requests
            .iter()
            .any(|req| req.id == request_id)
    }

    fn take_completed_request(&mut self, request_id: u32) -> Option<CompletedVfsRequest> {
        let index = self
            .completed_requests
            .iter()
            .position(|req| req.id == request_id)?;
        Some(self.completed_requests.remove(index))
    }

    fn alloc_request_id(&mut self) -> u32 {
        let id = self.next_request_id.max(1);
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        id
    }

    fn delete_key(&mut self, key: &str) -> Option<bool> {
        if self.files.remove(key).is_some() {
            Some(true)
        } else if self.async_entries.remove(key).is_some() {
            Some(true)
        } else if self.enabled {
            Some(false)
        } else {
            None
        }
    }

    fn move_key(&mut self, from_key: &str, to_key: String) -> Option<bool> {
        if let Some(data) = self.files.remove(from_key) {
            self.files.insert(to_key, data);
            Some(true)
        } else if let Some(entry) = self.async_entries.remove(from_key) {
            self.async_entries.insert(to_key, entry);
            Some(true)
        } else if self.enabled {
            Some(false)
        } else {
            None
        }
    }

    fn attributes_key(&self, key: &str) -> Option<u32> {
        if self.files.contains_key(key) || self.async_entries.contains_key(key) {
            return Some(0x80);
        }
        if self.directory_exists_key(key) {
            return Some(0x10);
        }
        if self.enabled {
            Some(INVALID_HANDLE_VALUE)
        } else {
            None
        }
    }

    fn find_entries_key(&self, dir_key: &str) -> Option<Vec<FindEntry>> {
        if !self.enabled {
            return None;
        }
        let prefix = if dir_key.ends_with('\\') {
            dir_key.to_string()
        } else {
            format!("{dir_key}\\")
        };
        let mut entries = Vec::new();
        for (key, data) in &self.files {
            push_virtual_find_entry(&mut entries, &prefix, key, 0x80, data.borrow().len() as u64);
        }
        for (key, entry) in &self.async_entries {
            push_virtual_find_entry(&mut entries, &prefix, key, 0x80, entry.size);
        }
        Some(entries)
    }

    fn directory_exists_key(&self, key: &str) -> bool {
        let prefix = if key.ends_with('\\') {
            key.to_string()
        } else {
            format!("{key}\\")
        };
        self.files.keys().any(|file| file.starts_with(&prefix))
            || self
                .async_entries
                .keys()
                .any(|file| file.starts_with(&prefix))
    }
}

impl Hle {
    pub fn set_drive_mount(&mut self, drive: char, path: PathBuf) {
        if let Some(idx) = drive_index(drive) {
            self.vfs.drive_table.set_mount(idx, path);
        }
    }

    pub fn set_drive_device(&mut self, drive: char, device: &str) {
        if let Some(idx) = drive_index(drive) {
            self.vfs
                .drive_table
                .set_device(idx, DriveDevice::from_app_db(device));
        }
    }

    pub fn set_virtual_drive_alias(&mut self, drive: char, target: &str, device: &str) {
        let Some(idx) = drive_index(drive) else {
            return;
        };
        let mut key = self.guest_path_key_no_alias(target);
        while key.ends_with('\\') && key.len() > 3 {
            key.pop();
        }
        self.vfs
            .drive_table
            .set_alias(idx, key, DriveDevice::from_app_db(device));
    }

    pub fn drive_is_mounted(&self, drive: char) -> bool {
        drive_index(drive).is_some_and(|idx| self.drive_mounted_at(idx))
    }

    fn drive_mounted_at(&self, idx: usize) -> bool {
        self.vfs.drive_table.mounted_at(idx)
    }

    fn drive_device_at(&self, idx: usize) -> DriveDevice {
        self.vfs.drive_table.device_at(idx)
    }

    fn drive_type(&self, drive: char) -> Option<u32> {
        let idx = drive_index(drive)?;
        self.drive_mounted_at(idx)
            .then(|| self.drive_device_at(idx).win32_drive_type((b'A' + idx as u8) as char))
    }

    fn drive_volume_name(&self, drive: char) -> &'static str {
        drive_index(drive)
            .map(|idx| self.drive_device_at(idx).volume_name())
            .unwrap_or("WEMU")
    }

    pub fn find_virtual_named_directory_near(&self, guest_path: &str, name: &str) -> Option<String> {
        let name = name.to_ascii_lowercase();
        let mut dir = parent_guest_key(&self.guest_path_key_no_alias(guest_path))?;
        loop {
            let candidate = join_guest_key(&dir, &name);
            if self.virtual_directory_exists_key(&candidate) {
                return Some(candidate);
            }
            let Some(parent) = parent_guest_key(&dir) else {
                break;
            };
            if parent == dir {
                break;
            }
            dir = parent;
        }
        None
    }

    pub fn set_cwd(&mut self, drive: char, path: String) {
        self.vfs.cwd_drive = drive.to_ascii_uppercase();
        self.vfs.cwd_path = if path.is_empty() {
            "\\".to_string()
        } else {
            path
        };
    }

    fn cwd_drive(&self) -> char {
        self.vfs.cwd_drive
    }

    fn cwd_path(&self) -> &str {
        &self.vfs.cwd_path
    }

    fn cwd_display(&self) -> String {
        format!("{}:{}", self.cwd_drive(), self.cwd_path())
    }

    fn full_guest_path(&self, raw: &str) -> String {
        GuestPath::resolve(raw, self.cwd_drive(), self.cwd_path()).display_path()
    }

    pub fn host_path_for_guest(&self, raw: &str) -> Result<PathBuf> {
        self.translate_raw_path(raw)
    }

    pub fn vfs_key_for_guest(&self, raw: &str) -> String {
        self.guest_path_key(raw)
    }

    pub fn enable_virtual_fs(&mut self) {
        self.vfs.enable();
    }

    fn virtual_fs_enabled(&self) -> bool {
        self.vfs.enabled
    }

    pub fn add_virtual_file(&mut self, guest_path: &str, bytes: &[u8]) {
        self.add_virtual_file_owned(guest_path, bytes.to_vec());
    }

    pub fn add_virtual_file_owned(&mut self, guest_path: &str, bytes: Vec<u8>) {
        self.vfs.enable();
        let key = self.guest_path_key_no_alias(guest_path);
        self.mark_virtual_drive_present_for_key(&key);
        let key = self.apply_virtual_drive_alias(key);
        self.vfs.insert_file(key, bytes);
    }

    pub fn add_async_virtual_file(&mut self, guest_path: &str, size: u64, writable: bool) {
        self.vfs.enable();
        let key = self.guest_path_key_no_alias(guest_path);
        self.mark_virtual_drive_present_for_key(&key);
        let key = self.apply_virtual_drive_alias(key);
        self.vfs.insert_async_file(key, size, writable);
    }

    pub fn enable_async_vfs_writes(&mut self) {
        self.vfs.enable_async_writes();
    }

    fn note_async_vfs_write(&mut self, key: &str, offset: u64, len: usize) {
        self.vfs.note_async_write(key, offset, len);
    }

    fn insert_virtual_memory_file(&mut self, key: String, data: Rc<RefCell<Vec<u8>>>) {
        self.vfs.files.insert(key, data);
    }

    pub fn pending_vfs_request_id(&self) -> u32 {
        self.vfs.pending_request_id()
    }

    pub fn pending_vfs_request_kind(&self) -> u32 {
        self.vfs.pending_request_kind()
    }

    pub fn pending_vfs_request_path(&self) -> &[u8] {
        self.vfs.pending_request_path()
    }

    pub fn pending_vfs_request_offset(&self) -> u64 {
        self.vfs.pending_request_offset()
    }

    pub fn pending_vfs_request_len(&self) -> u32 {
        self.vfs.pending_request_len()
    }

    pub fn pending_vfs_request_data(&self) -> &[u8] {
        self.vfs.pending_request_data()
    }

    pub fn complete_vfs_request(
        &mut self,
        request_id: u32,
        status: u32,
        transferred: u32,
        data: Vec<u8>,
    ) -> bool {
        self.vfs
            .complete_request(request_id, status, transferred, data)
    }

    fn open_virtual_file(&mut self, raw: &str, access: u32, creation: u32) -> VirtualOpen {
        let key = self.guest_path_key(raw);
        let wants_write = (access & 0x4000_0000) != 0;
        let async_existing = self.vfs.async_entries.get(&key).copied();
        let existing = self.vfs.files.get(&key).cloned();
        if existing.is_none() {
            if let Some(open) =
                self.open_async_virtual_file(&key, async_existing, wants_write, creation)
            {
                return open;
            }
        }
        let data = match (creation, existing) {
            (1, Some(_)) => return VirtualOpen::Failed(80),
            (1, None) if self.vfs.enabled && wants_write => {
                let data = Rc::new(RefCell::new(Vec::new()));
                self.vfs.files.insert(key.clone(), data.clone());
                data
            }
            (2, Some(data)) | (5, Some(data)) => {
                if wants_write {
                    data.borrow_mut().clear();
                }
                data
            }
            (2, None) | (4, None) if self.vfs.enabled && wants_write => {
                let data = Rc::new(RefCell::new(Vec::new()));
                self.vfs.files.insert(key.clone(), data.clone());
                data
            }
            (3, Some(data)) | (4, Some(data)) => data,
            (_, Some(data)) => data,
            _ if self.vfs.enabled => return VirtualOpen::Failed(2),
            _ => return VirtualOpen::Miss,
        };
        let handle = self.alloc_handle(Handle::File(FileHandle::memory(key, data, wants_write)));
        VirtualOpen::Opened(handle)
    }

    fn open_async_virtual_file(
        &mut self,
        key: &str,
        existing: Option<AsyncVfsEntry>,
        wants_write: bool,
        creation: u32,
    ) -> Option<VirtualOpen> {
        let can_create_for_write = self.vfs.async_writable && wants_write;
        let mut entry = match (creation, existing) {
            (1, Some(_)) => return Some(VirtualOpen::Failed(80)),
            (1, None) if can_create_for_write => AsyncVfsEntry {
                size: 0,
                writable: true,
            },
            (2, Some(mut entry)) | (5, Some(mut entry)) if can_create_for_write => {
                entry.size = 0;
                entry.writable = true;
                entry
            }
            (2, None) | (4, None) if can_create_for_write => AsyncVfsEntry {
                size: 0,
                writable: true,
            },
            (3, Some(entry)) | (4, Some(entry)) => {
                if wants_write && !entry.writable && !self.vfs.async_writable {
                    return Some(VirtualOpen::Failed(5));
                }
                AsyncVfsEntry {
                    writable: entry.writable || (wants_write && self.vfs.async_writable),
                    ..entry
                }
            }
            (_, Some(entry)) => entry,
            _ if self.vfs.enabled => return Some(VirtualOpen::Failed(2)),
            _ => return None,
        };
        if wants_write && self.vfs.async_writable {
            entry.writable = true;
        }
        self.vfs.async_entries.insert(key.to_string(), entry);
        let handle = self.alloc_handle(Handle::File(FileHandle::async_file(
            key.to_string(),
            entry.size,
            wants_write && entry.writable,
        )));
        Some(VirtualOpen::Opened(handle))
    }

    fn open_file_handle(&mut self, raw_name: &str, access: u32, creation: u32) -> FileOpen {
        let wants_write = (access & 0x4000_0000) != 0;
        let wants_read = (access & 0x8000_0000) != 0 || !wants_write;
        match self.open_virtual_file(raw_name, access, creation) {
            VirtualOpen::Opened(h) => return FileOpen::Opened(h),
            VirtualOpen::Failed(last_error) => return FileOpen::Failed(last_error),
            VirtualOpen::Miss => {}
        }
        let path = match self.translate_raw_path(raw_name) {
            Ok(path) => path,
            Err(_) => return FileOpen::Failed(2),
        };
        match open_host_file_candidates(raw_name, &path, wants_read, wants_write, creation) {
            Ok((file, _)) => {
                let h = self.alloc_handle(Handle::File(FileHandle::host(
                    self.guest_path_key(raw_name),
                    file,
                    wants_write,
                )));
                FileOpen::Opened(h)
            }
            Err((_, err)) => {
                if wants_write
                    && err.kind() == std::io::ErrorKind::NotFound
                    && flattened_legacy_root_path(raw_name, &path)
                        .and_then(|flat| case_insensitive_existing_path(&flat))
                        .is_some()
                {
                    let key = self.guest_path_key(raw_name);
                    let data = Rc::new(RefCell::new(Vec::new()));
                    self.insert_virtual_memory_file(key.clone(), data.clone());
                    let h = self.alloc_handle(Handle::File(FileHandle::memory(key, data, true)));
                    return FileOpen::Opened(h);
                }
                FileOpen::Failed(match err.kind() {
                    std::io::ErrorKind::NotFound => 2,
                    std::io::ErrorKind::AlreadyExists => 80,
                    _ => 5,
                })
            }
        }
    }

    fn begin_vfs_read(&mut self, key: &str, offset: u64, len: u32) -> u32 {
        self.vfs.begin_read(key, offset, len)
    }

    fn begin_vfs_write(&mut self, key: &str, offset: u64, data: Vec<u8>) -> u32 {
        self.vfs.begin_write(key, offset, data)
    }

    pub(crate) fn has_completed_vfs_request(&self, request_id: u32) -> bool {
        self.vfs.has_completed_request(request_id)
    }

    pub(crate) fn take_completed_vfs_request(
        &mut self,
        request_id: u32,
    ) -> Option<CompletedVfsRequest> {
        self.vfs.take_completed_request(request_id)
    }

    fn delete_virtual_file(&mut self, raw: &str) -> Option<bool> {
        let key = self.guest_path_key(raw);
        self.vfs.delete_key(&key)
    }

    fn move_virtual_file(&mut self, from: &str, to: &str) -> Option<bool> {
        let from_key = self.guest_path_key(from);
        let to_key = self.guest_path_key(to);
        self.vfs.move_key(&from_key, to_key)
    }

    fn virtual_file_attributes(&self, raw: &str) -> Option<u32> {
        let key = self.guest_path_key(raw);
        self.vfs.attributes_key(&key)
    }

    fn virtual_find_entries(&self, dir_raw: &str) -> Option<Vec<FindEntry>> {
        let dir_key = self.guest_path_key(dir_raw);
        self.vfs.find_entries_key(&dir_key)
    }

    fn guest_path_key(&self, raw: &str) -> String {
        self.apply_virtual_drive_alias(self.guest_path_key_no_alias(raw))
    }

    fn guest_path_key_no_alias(&self, raw: &str) -> String {
        GuestPath::resolve(raw, self.cwd_drive(), self.cwd_path()).key()
    }

    fn apply_virtual_drive_alias(&self, key: String) -> String {
        if key.len() < 3 || key.as_bytes()[1] != b':' {
            return key;
        }
        let Some(idx) = drive_index(key.as_bytes()[0] as char) else {
            return key;
        };
        let Some(alias) = self.vfs.drive_table.alias_at(idx) else {
            return key;
        };
        if key.len() == 3 {
            return alias.to_string();
        }
        join_guest_key(alias, key[3..].trim_start_matches('\\'))
    }

    fn mark_virtual_drive_present_for_key(&mut self, key: &str) {
        if key.len() >= 2 && key.as_bytes()[1] == b':' {
            if let Some(idx) = drive_index(key.as_bytes()[0] as char) {
                self.vfs.drive_table.mark_virtual_present(idx);
            }
        }
    }

    fn virtual_directory_exists_key(&self, key: &str) -> bool {
        self.vfs.directory_exists_key(key)
    }

    fn translate_raw_path(&self, raw: &str) -> Result<PathBuf> {
        let guest_path = GuestPath::resolve(raw, self.cwd_drive(), self.cwd_path());
        let drive = guest_path.drive();
        let Some(idx) = drive_index(drive) else {
            return Err(Error::Hle(format!("invalid drive {drive}: for {raw}")));
        };
        let root = self
            .vfs
            .drive_table
            .host_root_at(idx)
            .ok_or_else(|| Error::Hle(format!("drive {drive}: is not mounted for {raw}")))?;
        Ok(guest_path.append_to_host_root(root.clone()))
    }

    fn delete_file_path(&mut self, raw: &str) -> bool {
        if let Some(deleted) = self.delete_virtual_file(raw) {
            return deleted;
        }
        match self.translate_raw_path(raw) {
            Ok(path) => fs::remove_file(&path).is_ok(),
            Err(_) => {
                self.last_error = 2;
                false
            }
        }
    }

    fn move_file_path(&mut self, raw_from: &str, raw_to: Option<&str>, flags: u32) -> bool {
        const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
        let Some(raw_to) = raw_to else {
            return self.delete_file_path(raw_from);
        };
        if let Some(moved) = self.move_virtual_file(raw_from, raw_to) {
            return moved;
        }
        let Ok(from) = self.translate_raw_path(raw_from) else {
            self.last_error = 2;
            return false;
        };
        let Ok(to) = self.translate_raw_path(raw_to) else {
            self.last_error = 2;
            return false;
        };
        if (flags & MOVEFILE_REPLACE_EXISTING) != 0 && to.exists() {
            let _ = fs::remove_file(&to);
        }
        fs::rename(from, to).is_ok()
    }

    fn set_file_attributes_path(&mut self, raw: &str) -> bool {
        if let Some(attrs) = self.virtual_file_attributes(raw) {
            return attrs != INVALID_HANDLE_VALUE;
        }
        match self.translate_raw_path(raw) {
            Ok(path) => path.exists(),
            Err(_) => {
                self.last_error = 2;
                false
            }
        }
    }

    fn file_attribute_info(&mut self, raw: &str) -> Option<(u32, u64)> {
        if let Some(attrs) = self.virtual_file_attributes(raw) {
            return (attrs != INVALID_HANDLE_VALUE).then_some((attrs, 0));
        }
        let path = match self.translate_raw_path(raw) {
            Ok(path) => path,
            Err(_) => {
                self.last_error = 2;
                return None;
            }
        };
        match fs::metadata(&path) {
            Ok(md) => {
                let attrs = if md.is_dir() { 0x10 } else { 0x80 };
                Some((attrs, md.len()))
            }
            Err(_) => {
                self.last_error = 2;
                None
            }
        }
    }

    fn find_file_entries(&mut self, raw: &str) -> Option<FindEntriesResult> {
        let (dir_raw, pattern) = split_find_pattern(raw);
        let mut entries = self.virtual_find_entries(&dir_raw).unwrap_or_default();
        let host_dir = self.translate_raw_path(&dir_raw).ok();
        if let Some(dir) = host_dir.as_ref() {
            if let Ok(read_dir) = fs::read_dir(dir) {
                for entry in read_dir.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let Ok(meta) = entry.metadata() else {
                        continue;
                    };
                    push_find_entry_unique(
                        &mut entries,
                        FindEntry {
                            name,
                            attrs: if meta.is_dir() { 0x10 } else { 0x80 },
                            size: meta.len(),
                        },
                    );
                }
            }
        } else if entries.is_empty() && !self.virtual_fs_enabled() {
            self.last_error = 2;
            return None;
        }
        entries.retain(|entry| wildcard_match_ci(&pattern, &entry.name));
        entries.sort_by_key(|entry| entry.name.to_ascii_lowercase());
        if entries.is_empty() {
            self.last_error = 2;
            return None;
        }
        Some(FindEntriesResult {
            dir_raw,
            pattern,
            host_dir,
            entries,
        })
    }
}

fn drive_index(drive: char) -> Option<usize> {
    let drive = drive.to_ascii_uppercase();
    drive
        .is_ascii_alphabetic()
        .then_some(drive as usize - 'A' as usize)
}

fn parent_guest_key(key: &str) -> Option<String> {
    let trimmed = key.trim_end_matches('\\');
    if trimmed.len() <= 3 {
        return None;
    }
    let index = trimmed.rfind('\\')?;
    if index <= 2 {
        Some(trimmed[..3].to_string())
    } else {
        Some(trimmed[..index].to_string())
    }
}

fn join_guest_key(base: &str, child: &str) -> String {
    if base.ends_with('\\') {
        format!("{base}{child}")
    } else {
        format!("{base}\\{child}")
    }
}

fn push_virtual_find_entry(
    entries: &mut Vec<FindEntry>,
    dir_prefix: &str,
    key: &str,
    file_attrs: u32,
    file_size: u64,
) {
    let Some(rest) = key.strip_prefix(dir_prefix) else {
        return;
    };
    if rest.is_empty() {
        return;
    }
    let (name, attrs, size) = match rest.split_once('\\') {
        Some((name, _)) if !name.is_empty() => (name, 0x10, 0),
        Some(_) => return,
        None => (rest, file_attrs, file_size),
    };
    push_find_entry_unique(entries, FindEntry {
        name: name.to_string(),
        attrs,
        size,
    });
}

fn push_unique_path(out: &mut Vec<PathBuf>, path: PathBuf) {
    if !out.iter().any(|old| old == &path) {
        out.push(path);
    }
}

fn flattened_data_path(raw_name: &str, path: &Path) -> Option<PathBuf> {
    let raw = raw_name.replace('/', "\\");
    let rest = if raw.len() >= 2 && raw.as_bytes()[1] == b':' {
        &raw[2..]
    } else {
        raw.as_str()
    };
    let parts: Vec<&str> = rest
        .trim_start_matches('\\')
        .split('\\')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.len() < 2 || !parts[0].eq_ignore_ascii_case("data") {
        return None;
    }
    let mut base = path.to_path_buf();
    for _ in 0..parts.len() {
        base.pop();
    }
    for part in &parts[1..] {
        base.push(part);
    }
    Some(base)
}

fn flattened_legacy_root_path(raw_name: &str, path: &Path) -> Option<PathBuf> {
    let raw = raw_name.replace('/', "\\");
    let rest = if raw.len() >= 2 && raw.as_bytes()[1] == b':' {
        &raw[2..]
    } else {
        raw.as_str()
    };
    let parts: Vec<&str> = rest
        .trim_start_matches('\\')
        .split('\\')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.len() < 2 {
        return None;
    }
    let mut base = path.to_path_buf();
    for _ in 0..parts.len() {
        base.pop();
    }
    base.push(parts.last()?);
    Some(base)
}

fn case_insensitive_existing_path(path: &Path) -> Option<PathBuf> {
    use std::path::Component;

    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::CurDir | Component::ParentDir => {
                current.push(component.as_os_str());
            }
            Component::Normal(part) => {
                let dir = if current.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    current.as_path()
                };
                let wanted = part.to_string_lossy();
                let found = fs::read_dir(dir).ok()?.find_map(|entry| {
                    let entry = entry.ok()?;
                    let name = entry.file_name();
                    name.to_string_lossy()
                        .eq_ignore_ascii_case(&wanted)
                        .then_some(name)
                })?;
                current.push(found);
            }
        }
    }
    current.exists().then_some(current)
}

fn split_find_pattern(raw: &str) -> (String, String) {
    let raw = raw.replace('/', "\\");
    let (dir, pattern) = match raw.rfind('\\') {
        Some(0) => ("\\".to_string(), raw[1..].to_string()),
        Some(pos) => (raw[..pos].to_string(), raw[pos + 1..].to_string()),
        None => (".".to_string(), raw),
    };
    let pattern = if pattern.is_empty() || pattern == "*.*" {
        "*".to_string()
    } else {
        pattern
    };
    (dir, pattern)
}

fn open_host_file_candidates(
    raw_name: &str,
    path: &Path,
    wants_read: bool,
    wants_write: bool,
    creation: u32,
) -> std::result::Result<(File, PathBuf), (PathBuf, std::io::Error)> {
    let mut last_err = None;
    for candidate in host_file_candidates(raw_name, path, wants_write) {
        let mut opts = OpenOptions::new();
        opts.read(wants_read).write(wants_write);
        match creation {
            1 => {
                opts.create_new(true);
            }
            2 => {
                opts.create(true).truncate(true);
            }
            3 => {}
            4 => {
                if wants_write {
                    opts.create(true);
                }
            }
            5 => {
                if wants_write {
                    opts.truncate(true);
                }
            }
            _ => {}
        }
        match opts.open(&candidate) {
            Ok(file) => return Ok((file, candidate)),
            Err(err) => last_err = Some((candidate, err)),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        (
            path.to_path_buf(),
            std::io::Error::new(std::io::ErrorKind::NotFound, "no path candidates"),
        )
    }))
}

fn host_file_candidates(raw_name: &str, path: &Path, wants_write: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    push_unique_path(&mut out, path.to_path_buf());
    if let Some(path) = case_insensitive_existing_path(path) {
        push_unique_path(&mut out, path);
    }
    if !wants_write {
        if let Some(flat) = flattened_data_path(raw_name, path) {
            push_unique_path(&mut out, flat.clone());
            if let Some(path) = case_insensitive_existing_path(&flat) {
                push_unique_path(&mut out, path);
            }
        }
        if let Some(flat) = flattened_legacy_root_path(raw_name, path) {
            push_unique_path(&mut out, flat.clone());
            if let Some(path) = case_insensitive_existing_path(&flat) {
                push_unique_path(&mut out, path);
            }
        }
    }
    out
}

fn push_find_entry_unique(entries: &mut Vec<FindEntry>, entry: FindEntry) {
    if entries
        .iter()
        .any(|old| old.name.eq_ignore_ascii_case(&entry.name))
    {
        return;
    }
    entries.push(entry);
}
