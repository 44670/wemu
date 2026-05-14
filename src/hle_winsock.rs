const WSADATA_SIZE: usize = 398;
const WINSOCK_VERSION_1_1: u16 = 0x0101;
const SOCKET_ERROR: u32 = 0xffff_ffff;
const WSAEFAULT: u32 = 10014;
const WSAEINVAL: u32 = 10022;
const WSAEWOULDBLOCK: u32 = 10035;
const WSAENOTSOCK: u32 = 10038;
const WSAHOST_NOT_FOUND: u32 = 11001;

fn callback_for_winsock(name: &str) -> Option<HleCallback> {
    match name {
        "#1" => Some(hle_accept),
        "#2" => Some(hle_bind),
        "#3" => Some(hle_closesocket),
        "#6" => Some(hle_getsockname),
        "#7" => Some(hle_getsockopt),
        "#8" => Some(hle_htonl),
        "#9" => Some(hle_htons),
        "#10" => Some(hle_inet_addr),
        "#11" => Some(hle_inet_ntoa),
        "#12" => Some(hle_ioctlsocket),
        "#14" => Some(hle_listen),
        "#15" => Some(hle_ntohs),
        "#16" => Some(hle_recv),
        "#17" => Some(hle_recvfrom),
        "#19" => Some(hle_send),
        "#20" => Some(hle_sendto),
        "#21" => Some(hle_setsockopt),
        "#23" => Some(hle_socket),
        "#52" => Some(hle_gethostbyname),
        "#57" => Some(hle_gethostname),
        "#101" => Some(hle_wsa_async_select),
        "#102" => Some(hle_wsa_async_get_host_by_addr),
        "#103" => Some(hle_wsa_async_get_host_by_name),
        "#108" => Some(hle_wsa_cancel_async_request),
        "#111" => Some(hle_wsa_get_last_error),
        "#112" => Some(hle_wsa_set_last_error),
        "#115" => Some(hle_wsa_startup),
        "#116" => Some(hle_wsa_cleanup),
        "accept" => Some(hle_accept),
        "bind" => Some(hle_bind),
        "closesocket" => Some(hle_closesocket),
        "getsockname" => Some(hle_getsockname),
        "getsockopt" => Some(hle_getsockopt),
        "inet_ntoa" => Some(hle_inet_ntoa),
        "listen" => Some(hle_listen),
        "ntohs" => Some(hle_ntohs),
        "recv" => Some(hle_recv),
        "recvfrom" => Some(hle_recvfrom),
        "send" => Some(hle_send),
        "sendto" => Some(hle_sendto),
        "setsockopt" => Some(hle_setsockopt),
        "socket" => Some(hle_socket),
        "gethostname" => Some(hle_gethostname),
        "WSAAsyncSelect" => Some(hle_wsa_async_select),
        "WSAAsyncGetHostByAddr" => Some(hle_wsa_async_get_host_by_addr),
        "WSAAsyncGetHostByName" => Some(hle_wsa_async_get_host_by_name),
        "WSACancelAsyncRequest" => Some(hle_wsa_cancel_async_request),
        _ => None,
    }
}

// int WSAStartup(WORD wVersionRequested, LPWSADATA lpWSAData)
// Initialize a minimal Winsock 1.1 provider and fill WSADATA when supplied.
fn hle_wsa_startup(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 1);
    if out == 0 {
        emu.hle.wsa_last_error = WSAEFAULT;
        ret(emu, WSAEFAULT);
        return HleResult::Retn(8);
    }
    write_wsa_data(emu, out);
    emu.hle.wsa_last_error = 0;
    ret(emu, 0);
    HleResult::Retn(8)
}

// int WSACleanup(void)
// Accept teardown of the minimal Winsock provider.
fn hle_wsa_cleanup(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.wsa_last_error = 0;
    ret(emu, 0);
    HleResult::Retn(0)
}

// int WSAGetLastError(void)
// Return the Winsock-specific last error value.
fn hle_wsa_get_last_error(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, emu.hle.wsa_last_error);
    HleResult::Retn(0)
}

// void WSASetLastError(int iError)
// Store the Winsock-specific last error value.
fn hle_wsa_set_last_error(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.wsa_last_error = arg(emu, 0);
    HleResult::Retn(4)
}

// u_long htonl(u_long hostlong)
// Convert a 32-bit integer from host byte order to network byte order.
fn hle_htonl(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    ret(emu, arg(emu, 0).swap_bytes());
    HleResult::Retn(4)
}

// u_short htons(u_short hostshort)
// Convert a 16-bit integer from host byte order to network byte order.
fn hle_htons(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = (arg(emu, 0) as u16).swap_bytes() as u32;
    ret(emu, value);
    HleResult::Retn(4)
}

// u_short ntohs(u_short netshort)
// Convert a 16-bit integer from network byte order to host byte order.
fn hle_ntohs(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let value = (arg(emu, 0) as u16).swap_bytes() as u32;
    ret(emu, value);
    HleResult::Retn(4)
}

// unsigned long inet_addr(const char *cp)
// Parse dotted-quad IPv4 text and return a network-order address value.
fn hle_inet_addr(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let cp = arg(emu, 0);
    let value = if cp == 0 {
        None
    } else {
        emu.memory
            .cstr_lossy(cp, 64)
            .ok()
            .and_then(|text| parse_ipv4_dotted_quad(&text))
    };
    match value {
        Some(addr) => {
            emu.hle.wsa_last_error = 0;
            ret(emu, addr);
        }
        None => {
            ret(emu, SOCKET_ERROR);
        }
    }
    HleResult::Retn(4)
}

// char *inet_ntoa(struct in_addr in)
// Format an IPv4 address into Winsock's reusable static string buffer.
fn hle_inet_ntoa(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let addr = arg(emu, 0);
    if emu.hle.winsock_inet_ntoa_buffer == 0 {
        emu.hle.winsock_inet_ntoa_buffer = emu
            .hle
            .alloc_private(&mut emu.memory, 16, PagePerm::READ | PagePerm::WRITE)
            .hle();
    }
    let octets = addr.to_le_bytes();
    let text = format!("{}.{}.{}.{}", octets[0], octets[1], octets[2], octets[3]);
    emu.memory
        .write_cstr(emu.hle.winsock_inet_ntoa_buffer, &text, 16)
        .hle();
    emu.hle.wsa_last_error = 0;
    ret(emu, emu.hle.winsock_inet_ntoa_buffer);
    HleResult::Retn(4)
}

// int ioctlsocket(SOCKET s, long cmd, u_long *argp)
// Accept socket mode probes without backing host sockets.
fn hle_ioctlsocket(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.wsa_last_error = 0;
    ret(emu, 0);
    HleResult::Retn(12)
}

// SOCKET socket(int af, int type, int protocol)
// Allocate a fake socket so network-capable games can disable or probe multiplayer paths.
fn hle_socket(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let handle = emu.hle.alloc_handle(Handle::Socket);
    emu.hle.wsa_last_error = 0;
    ret(emu, handle);
    HleResult::Retn(12)
}

// SOCKET accept(SOCKET s, struct sockaddr *addr, int *addrlen)
// Report that fake nonblocking sockets have no pending inbound connection.
fn hle_accept(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let socket = arg(emu, 0);
    emu.hle.wsa_last_error = if is_fake_socket(emu, socket) {
        WSAEWOULDBLOCK
    } else {
        WSAENOTSOCK
    };
    ret(emu, SOCKET_ERROR);
    HleResult::Retn(12)
}

// int bind(SOCKET s, const struct sockaddr *name, int namelen)
// Accept local endpoint binding for fake sockets without touching host networking.
fn hle_bind(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let socket = arg(emu, 0);
    if !is_fake_socket(emu, socket) {
        emu.hle.wsa_last_error = WSAENOTSOCK;
        ret(emu, SOCKET_ERROR);
    } else {
        emu.hle.wsa_last_error = 0;
        ret(emu, 0);
    }
    HleResult::Retn(12)
}

// int listen(SOCKET s, int backlog)
// Accept listen state for fake sockets so multiplayer probes can continue.
fn hle_listen(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let socket = arg(emu, 0);
    if !is_fake_socket(emu, socket) {
        emu.hle.wsa_last_error = WSAENOTSOCK;
        ret(emu, SOCKET_ERROR);
    } else {
        emu.hle.wsa_last_error = 0;
        ret(emu, 0);
    }
    HleResult::Retn(8)
}

// int closesocket(SOCKET s)
// Close a fake socket handle.
fn hle_closesocket(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let socket = arg(emu, 0);
    if emu.hle.close_handle(socket) {
        emu.hle.wsa_last_error = 0;
        ret(emu, 0);
    } else {
        emu.hle.wsa_last_error = WSAENOTSOCK;
        ret(emu, SOCKET_ERROR);
    }
    HleResult::Retn(4)
}

// int getsockname(SOCKET s, struct sockaddr *name, int *namelen)
// Return a zero IPv4 sockaddr for fake sockets.
fn hle_getsockname(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let socket = arg(emu, 0);
    let name = arg(emu, 1);
    let name_len = arg(emu, 2);
    if !is_fake_socket(emu, socket) {
        emu.hle.wsa_last_error = WSAENOTSOCK;
        ret(emu, SOCKET_ERROR);
        return HleResult::Retn(12);
    }
    if name == 0 || name_len == 0 {
        emu.hle.wsa_last_error = WSAEFAULT;
        ret(emu, SOCKET_ERROR);
        return HleResult::Retn(12);
    }
    let len = emu.memory.read_u32(name_len).unwrap_or(0).min(16);
    if len >= 2 {
        emu.memory.write_u16(name, 2).hle(); // AF_INET
    }
    if len > 2 {
        emu.memory
            .write_bytes(name + 2, &vec![0; (len - 2) as usize])
            .hle();
    }
    emu.memory.write_u32(name_len, 16).hle();
    emu.hle.wsa_last_error = 0;
    ret(emu, 0);
    HleResult::Retn(12)
}

// int getsockopt(SOCKET s, int level, int optname, char *optval, int *optlen)
// Return zero-valued option data for fake sockets.
fn hle_getsockopt(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let socket = arg(emu, 0);
    let optval = arg(emu, 3);
    let optlen = arg(emu, 4);
    if !is_fake_socket(emu, socket) {
        emu.hle.wsa_last_error = WSAENOTSOCK;
        ret(emu, SOCKET_ERROR);
        return HleResult::Retn(20);
    }
    if optval != 0 && optlen != 0 {
        let len = emu.memory.read_u32(optlen).unwrap_or(0).min(16);
        if len != 0 {
            emu.memory.memset(optval, 0, len).hle();
        }
    }
    emu.hle.wsa_last_error = 0;
    ret(emu, 0);
    HleResult::Retn(20)
}

// int send(SOCKET s, const char *buf, int len, int flags)
// Pretend outgoing bytes were accepted without touching the host network.
fn hle_send(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let socket = arg(emu, 0);
    let len = arg(emu, 2);
    if !is_fake_socket(emu, socket) {
        emu.hle.wsa_last_error = WSAENOTSOCK;
        ret(emu, SOCKET_ERROR);
    } else {
        emu.hle.wsa_last_error = 0;
        ret(emu, len);
    }
    HleResult::Retn(16)
}

// int sendto(SOCKET s, const char *buf, int len, int flags, const struct sockaddr *to, int tolen)
// Pretend datagram bytes were accepted without touching the host network.
fn hle_sendto(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let socket = arg(emu, 0);
    let len = arg(emu, 2);
    if !is_fake_socket(emu, socket) {
        emu.hle.wsa_last_error = WSAENOTSOCK;
        ret(emu, SOCKET_ERROR);
    } else {
        emu.hle.wsa_last_error = 0;
        ret(emu, len);
    }
    HleResult::Retn(24)
}

// int recv(SOCKET s, char *buf, int len, int flags)
// Report non-blocking no-data for fake sockets.
fn hle_recv(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let socket = arg(emu, 0);
    emu.hle.wsa_last_error = if is_fake_socket(emu, socket) {
        WSAEWOULDBLOCK
    } else {
        WSAENOTSOCK
    };
    ret(emu, SOCKET_ERROR);
    HleResult::Retn(16)
}

// int recvfrom(SOCKET s, char *buf, int len, int flags, struct sockaddr *from, int *fromlen)
// Report non-blocking no-data for fake sockets.
fn hle_recvfrom(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let socket = arg(emu, 0);
    emu.hle.wsa_last_error = if is_fake_socket(emu, socket) {
        WSAEWOULDBLOCK
    } else {
        WSAENOTSOCK
    };
    ret(emu, SOCKET_ERROR);
    HleResult::Retn(24)
}

// int setsockopt(SOCKET s, int level, int optname, const char *optval, int optlen)
// Accept socket option writes for fake sockets.
fn hle_setsockopt(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let socket = arg(emu, 0);
    if !is_fake_socket(emu, socket) {
        emu.hle.wsa_last_error = WSAENOTSOCK;
        ret(emu, SOCKET_ERROR);
    } else {
        emu.hle.wsa_last_error = 0;
        ret(emu, 0);
    }
    HleResult::Retn(20)
}

// struct hostent *gethostbyname(const char *name)
// Report no host database entry while keeping Winsock error state coherent.
fn hle_gethostbyname(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.wsa_last_error = WSAHOST_NOT_FOUND;
    ret(emu, 0);
    HleResult::Retn(4)
}

// int WSAAsyncSelect(SOCKET s, HWND hwnd, unsigned int msg, long event)
// Accept async notification registration for fake sockets.
fn hle_wsa_async_select(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let socket = arg(emu, 0);
    if !is_fake_socket(emu, socket) {
        emu.hle.wsa_last_error = WSAENOTSOCK;
        ret(emu, SOCKET_ERROR);
    } else {
        emu.hle.wsa_last_error = 0;
        ret(emu, 0);
    }
    HleResult::Retn(16)
}

// HANDLE WSAAsyncGetHostByAddr(HWND hwnd, unsigned int msg, const char *addr, int len, int type, char *buf, int buflen)
// Report no host database result without scheduling host network work.
fn hle_wsa_async_get_host_by_addr(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.wsa_last_error = WSAHOST_NOT_FOUND;
    ret(emu, 0);
    HleResult::Retn(28)
}

// HANDLE WSAAsyncGetHostByName(HWND hwnd, unsigned int msg, const char *name, char *buf, int buflen)
// Report no host database result without scheduling host network work.
fn hle_wsa_async_get_host_by_name(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.wsa_last_error = WSAHOST_NOT_FOUND;
    ret(emu, 0);
    HleResult::Retn(20)
}

// int WSACancelAsyncRequest(HANDLE task)
// Accept cancellation of fake or already-failed async lookup requests.
fn hle_wsa_cancel_async_request(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    emu.hle.wsa_last_error = 0;
    ret(emu, 0);
    HleResult::Retn(4)
}

// int gethostname(char *name, int namelen)
// Return a stable local hostname for fake Winsock.
fn hle_gethostname(emu: &mut Emulator, _: &HleEntry) -> HleResult {
    let out = arg(emu, 0);
    let len = arg(emu, 1) as usize;
    if out == 0 || len == 0 {
        emu.hle.wsa_last_error = WSAEFAULT;
        ret(emu, SOCKET_ERROR);
        return HleResult::Retn(8);
    }
    if len < 5 {
        emu.hle.wsa_last_error = WSAEINVAL;
        ret(emu, SOCKET_ERROR);
        return HleResult::Retn(8);
    }
    emu.memory.write_cstr(out, "wemu", len).hle();
    emu.hle.wsa_last_error = 0;
    ret(emu, 0);
    HleResult::Retn(8)
}

fn is_fake_socket(emu: &mut Emulator, socket: u32) -> bool {
    matches!(emu.hle.handle_mut(socket), Some(Handle::Socket))
}

fn write_wsa_data(emu: &mut Emulator, out: u32) {
    let mut data = [0u8; WSADATA_SIZE];
    data[0..2].copy_from_slice(&WINSOCK_VERSION_1_1.to_le_bytes());
    data[2..4].copy_from_slice(&WINSOCK_VERSION_1_1.to_le_bytes());
    write_fixed_ascii(&mut data[4..261], "WEMU Winsock 1.1");
    write_fixed_ascii(&mut data[261..390], "Running");
    data[390..392].copy_from_slice(&128u16.to_le_bytes());
    data[392..394].copy_from_slice(&1024u16.to_le_bytes());
    emu.memory.write_bytes(out, &data).hle();
}

fn write_fixed_ascii(dst: &mut [u8], value: &str) {
    let bytes = value.as_bytes();
    let len = bytes.len().min(dst.len().saturating_sub(1));
    dst[..len].copy_from_slice(&bytes[..len]);
}

fn parse_ipv4_dotted_quad(text: &str) -> Option<u32> {
    let mut octets = [0u8; 4];
    let mut count = 0;
    for part in text.split('.') {
        if count == 4 || part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        octets[count] = part.parse::<u8>().ok()?;
        count += 1;
    }
    (count == 4).then(|| u32::from_le_bytes(octets))
}
