use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuestPath {
    drive: char,
    components: Vec<String>,
}

impl GuestPath {
    pub fn resolve(raw: &str, cwd_drive: char, cwd_path: &str) -> Self {
        let path = raw.replace('/', "\\");
        let mut drive = cwd_drive.to_ascii_uppercase();
        let mut rest = path.as_str();
        let absolute_drive = path.len() >= 2 && path.as_bytes()[1] == b':';
        if absolute_drive {
            drive = path.as_bytes()[0].to_ascii_uppercase() as char;
            rest = &path[2..];
        }

        let mut components = Vec::new();
        if !rest.starts_with('\\') && !absolute_drive {
            push_guest_components(&mut components, cwd_path);
        }
        push_guest_components(&mut components, rest);

        Self { drive, components }
    }

    pub fn drive(&self) -> char {
        self.drive
    }

    pub fn components(&self) -> &[String] {
        &self.components
    }

    pub fn display_path(&self) -> String {
        if self.components.is_empty() {
            format!("{}:\\", self.drive)
        } else {
            format!("{}:\\{}", self.drive, self.components.join("\\"))
        }
    }

    pub fn key(&self) -> String {
        self.display_path().to_ascii_lowercase()
    }

    pub fn parent_dir(&self) -> (char, String) {
        let parent_components = self
            .components
            .split_last()
            .map_or(&[][..], |(_, parent)| parent);
        let path = if parent_components.is_empty() {
            "\\".to_string()
        } else {
            format!("\\{}", parent_components.join("\\"))
        };
        (self.drive, path)
    }

    pub fn append_to_host_root(&self, mut root: PathBuf) -> PathBuf {
        for component in &self.components {
            root.push(component);
        }
        root
    }
}

fn push_guest_components(out: &mut Vec<String>, path: &str) {
    for part in path
        .trim_start_matches('\\')
        .split('\\')
        .filter(|part| !part.is_empty() && *part != ".")
    {
        if part == ".." {
            out.pop();
        } else {
            out.push(part.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GuestPath;

    #[test]
    fn resolves_absolute_and_relative_paths() {
        assert_eq!(
            GuestPath::resolve("C:/Data/./File.TXT", 'D', "\\OLD").display_path(),
            "C:\\Data\\File.TXT"
        );
        assert_eq!(
            GuestPath::resolve("file.txt", 'C', "\\Data").display_path(),
            "C:\\Data\\file.txt"
        );
        assert_eq!(
            GuestPath::resolve("..\\file.txt", 'C', "\\Data\\Sub").display_path(),
            "C:\\Data\\file.txt"
        );
    }

    #[test]
    fn preserves_existing_drive_relative_behavior() {
        assert_eq!(
            GuestPath::resolve("D:file.txt", 'C', "\\Data").display_path(),
            "D:\\file.txt"
        );
    }

    #[test]
    fn exposes_lowercase_lookup_key_and_parent() {
        let path = GuestPath::resolve("C:\\Data\\File.TXT", 'D', "\\");
        assert_eq!(path.key(), "c:\\data\\file.txt");
        assert_eq!(path.parent_dir(), ('C', "\\Data".to_string()));
        assert_eq!(
            GuestPath::resolve("C:\\File.TXT", 'D', "\\").parent_dir(),
            ('C', "\\".to_string())
        );
    }
}
