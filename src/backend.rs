#[cfg(feature = "sdl2")]
use std::io::{Read, Write};
#[cfg(feature = "sdl2")]
use std::net::{TcpListener, TcpStream};
use std::path::Path;
#[cfg(all(feature = "sdl2", unix))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "sdl2")]
use std::sync::mpsc::{self, Receiver, Sender};
#[cfg(all(feature = "sdl2", unix))]
use std::sync::Once;
#[cfg(feature = "sdl2")]
use std::thread;
#[cfg(feature = "sdl2")]
use std::time::Duration;

use crate::Error;
use crate::{png, Result};

#[cfg(all(feature = "sdl2", unix))]
const SIGUSR1: i32 = 10;
#[cfg(feature = "sdl2")]
const SDL_SIGNAL_SCREENSHOT: &str = "/tmp/wemu.png";

#[cfg(all(feature = "sdl2", unix))]
static SDL_SIGNAL_SCREENSHOT_REQUESTED: AtomicBool = AtomicBool::new(false);
#[cfg(all(feature = "sdl2", unix))]
static SDL_SIGNAL_HANDLER_ONCE: Once = Once::new();

#[cfg(all(feature = "sdl2", unix))]
extern "C" fn request_sdl_signal_screenshot(_: i32) {
    SDL_SIGNAL_SCREENSHOT_REQUESTED.store(true, Ordering::SeqCst);
}

#[cfg(all(feature = "sdl2", unix))]
unsafe extern "C" {
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
}

#[cfg(all(feature = "sdl2", unix))]
fn install_sdl_signal_screenshot_handler() {
    SDL_SIGNAL_HANDLER_ONCE.call_once(|| unsafe {
        signal(SIGUSR1, request_sdl_signal_screenshot);
    });
}

#[cfg(all(feature = "sdl2", not(unix)))]
fn install_sdl_signal_screenshot_handler() {}

#[cfg(all(feature = "sdl2", unix))]
fn take_sdl_signal_screenshot_request() -> bool {
    SDL_SIGNAL_SCREENSHOT_REQUESTED.swap(false, Ordering::SeqCst)
}

#[cfg(all(feature = "sdl2", not(unix)))]
fn take_sdl_signal_screenshot_request() -> bool {
    false
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendEvent {
    Quit,
    MouseMove { x: u32, y: u32 },
    MouseButtonDown { x: u32, y: u32 },
    MouseButtonUp { x: u32, y: u32 },
    MouseRightButtonDown { x: u32, y: u32 },
    MouseRightButtonUp { x: u32, y: u32 },
    KeyDown { vk: u32 },
    KeyUp { vk: u32 },
    TextInput { text: String },
    Text { text: String },
}

pub trait Backend {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn framebuffer(&self) -> &[u8];
    fn framebuffer_mut(&mut self) -> &mut [u8];
    fn resize(&mut self, width: u32, height: u32) -> Result<()>;
    fn present_bgra(
        &mut self,
        src: &[u8],
        width: u32,
        height: u32,
        pitch: u32,
        bpp: u32,
    ) -> Result<()>;
    fn present(&mut self) -> Result<()> {
        Ok(())
    }
    fn wants_event_poll(&self) -> bool {
        false
    }
    fn uses_wall_clock(&self) -> bool {
        self.wants_event_poll()
    }
    fn poll_events(&mut self) -> Result<Vec<BackendEvent>> {
        Ok(Vec::new())
    }
    fn write_png(&self, path: &Path) -> Result<()>;
}

pub struct HeadlessBackend {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    wall_clock: bool,
}

impl HeadlessBackend {
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_clock(width, height, false)
    }

    pub fn new_live(width: u32, height: u32) -> Self {
        Self::with_clock(width, height, true)
    }

    fn with_clock(width: u32, height: u32, wall_clock: bool) -> Self {
        let rgba = black_framebuffer(width, height).unwrap_or_default();
        Self {
            width,
            height,
            rgba,
            wall_clock,
        }
    }
}

impl Backend for HeadlessBackend {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn framebuffer(&self) -> &[u8] {
        &self.rgba
    }

    fn framebuffer_mut(&mut self) -> &mut [u8] {
        &mut self.rgba
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return Ok(());
        }
        self.rgba = black_framebuffer(width, height)?;
        self.width = width;
        self.height = height;
        Ok(())
    }

    fn present_bgra(
        &mut self,
        src: &[u8],
        width: u32,
        height: u32,
        pitch: u32,
        bpp: u32,
    ) -> Result<()> {
        copy_bgra_to_rgba(
            &mut self.rgba,
            self.width,
            self.height,
            src,
            width,
            height,
            pitch,
            bpp,
        );
        Ok(())
    }

    fn uses_wall_clock(&self) -> bool {
        self.wall_clock
    }

    fn write_png(&self, path: &Path) -> Result<()> {
        png::write_rgba_png(path, self.width, self.height, &self.rgba)
    }
}

fn black_framebuffer(width: u32, height: u32) -> Result<Vec<u8>> {
    let len = framebuffer_len(width.max(1), height.max(1))?;
    let mut rgba = vec![0; len];
    for px in rgba.chunks_mut(4) {
        px.copy_from_slice(&[0, 0, 0, 255]);
    }
    Ok(rgba)
}

fn framebuffer_len(width: u32, height: u32) -> Result<usize> {
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .map(|bytes| bytes as usize)
        .ok_or_else(|| Error::Hle(format!("framebuffer size overflow {width}x{height}")))
}

fn copy_bgra_to_rgba(
    rgba: &mut [u8],
    dst_width: u32,
    dst_height: u32,
    src: &[u8],
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
) {
    let copy_w = dst_width.min(width) as usize;
    let copy_h = dst_height.min(height) as usize;
    match bpp {
        8 => {
            for y in 0..copy_h {
                let row = &src[(y * pitch as usize)..];
                for (x, value) in row.iter().take(copy_w).enumerate() {
                    let dst = (y * dst_width as usize + x) * 4;
                    rgba[dst] = *value;
                    rgba[dst + 1] = *value;
                    rgba[dst + 2] = *value;
                    rgba[dst + 3] = 255;
                }
            }
        }
        16 | 15 => {
            for y in 0..copy_h {
                let row = &src[(y * pitch as usize)..];
                for x in 0..copy_w {
                    let v = u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]);
                    let r = ((v >> 11) & 0x1f) as u8;
                    let g = ((v >> 5) & 0x3f) as u8;
                    let b = (v & 0x1f) as u8;
                    let dst = (y * dst_width as usize + x) * 4;
                    rgba[dst] = (r << 3) | (r >> 2);
                    rgba[dst + 1] = (g << 2) | (g >> 4);
                    rgba[dst + 2] = (b << 3) | (b >> 2);
                    rgba[dst + 3] = 255;
                }
            }
        }
        24 => {
            for y in 0..copy_h {
                let row = &src[(y * pitch as usize)..];
                for x in 0..copy_w {
                    let dst = (y * dst_width as usize + x) * 4;
                    rgba[dst] = row[x * 3 + 2];
                    rgba[dst + 1] = row[x * 3 + 1];
                    rgba[dst + 2] = row[x * 3];
                    rgba[dst + 3] = 255;
                }
            }
        }
        _ => {
            for y in 0..copy_h {
                let row = &src[(y * pitch as usize)..];
                for x in 0..copy_w {
                    let dst = (y * dst_width as usize + x) * 4;
                    rgba[dst] = row[x * 4 + 2];
                    rgba[dst + 1] = row[x * 4 + 1];
                    rgba[dst + 2] = row[x * 4];
                    rgba[dst + 3] = 255;
                }
            }
        }
    }
}

#[cfg(feature = "sdl2")]
pub struct SdlBackend {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    _sdl: sdl2::Sdl,
    event_pump: sdl2::EventPump,
    // SDL textures are tied to the renderer. Store the texture before the
    // canvas so it is destroyed before the renderer at shutdown.
    texture: sdl2::render::Texture<'static>,
    canvas: sdl2::render::Canvas<sdl2::video::Window>,
    ws: Option<SdlWsControl>,
}

#[cfg(feature = "sdl2")]
impl SdlBackend {
    pub fn new(width: u32, height: u32, ws_addr: Option<&str>) -> Result<Self> {
        install_sdl_signal_screenshot_handler();
        let ws_addr = ws_addr
            .map(str::to_string)
            .or_else(|| std::env::var("WEMU_SDL_WS").ok());
        let sdl = sdl2::init().map_err(sdl_error)?;
        let video = sdl.video().map_err(sdl_error)?;
        let window = video
            .window("wemu", width, height)
            .position_centered()
            .build()
            .map_err(sdl_error)?;
        let mut canvas = window.into_canvas().software().build().map_err(sdl_error)?;
        canvas.set_logical_size(width, height).map_err(sdl_error)?;
        let texture_creator = canvas.texture_creator();
        let texture = texture_creator
            .create_texture_streaming(sdl2::pixels::PixelFormatEnum::ABGR8888, width, height)
            .map_err(sdl_error)?;
        // The texture lifetime in rust-sdl2 models the renderer relationship.
        // It does not borrow the local TextureCreator after construction; the
        // field order above makes the texture drop before the canvas/renderer.
        let texture = unsafe {
            std::mem::transmute::<sdl2::render::Texture<'_>, sdl2::render::Texture<'static>>(
                texture,
            )
        };
        let event_pump = sdl.event_pump().map_err(sdl_error)?;
        let rgba = black_framebuffer(width, height)?;
        Ok(Self {
            width,
            height,
            rgba,
            _sdl: sdl,
            event_pump,
            texture,
            canvas,
            ws: ws_addr.as_deref().map(start_sdl_ws_control).transpose()?,
        })
    }

    fn sdl_xy(width: u32, height: u32, x: i32, y: i32) -> (u32, u32) {
        let max_x = width.saturating_sub(1) as i32;
        let max_y = height.saturating_sub(1) as i32;
        (x.clamp(0, max_x) as u32, y.clamp(0, max_y) as u32)
    }

    fn maybe_write_signal_screenshot(&self) {
        if take_sdl_signal_screenshot_request() {
            let path = Path::new(SDL_SIGNAL_SCREENSHOT);
            match self.write_png(path) {
                Ok(()) => eprintln!("wemu: wrote {SDL_SIGNAL_SCREENSHOT} after SIGUSR1"),
                Err(err) => eprintln!("wemu: failed to write {SDL_SIGNAL_SCREENSHOT}: {err}"),
            }
        }
    }

    fn service_ws_commands(&mut self, events: &mut Vec<BackendEvent>) {
        let Some(ws) = &self.ws else {
            return;
        };
        while let Ok(cmd) = ws.rx.try_recv() {
            match cmd {
                SdlWsCommand::Event(event) => events.push(event),
                SdlWsCommand::Screenshot { reply } => {
                    let result = png::encode_rgba_png(self.width, self.height, &self.rgba);
                    let _ = reply.send(result);
                }
            }
        }
    }
}

#[cfg(feature = "sdl2")]
impl Backend for SdlBackend {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn framebuffer(&self) -> &[u8] {
        &self.rgba
    }

    fn framebuffer_mut(&mut self) -> &mut [u8] {
        &mut self.rgba
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return Ok(());
        }
        self.canvas
            .window_mut()
            .set_size(width, height)
            .map_err(sdl_error)?;
        self.canvas
            .set_logical_size(width, height)
            .map_err(sdl_error)?;
        let texture_creator = self.canvas.texture_creator();
        let texture = texture_creator
            .create_texture_streaming(sdl2::pixels::PixelFormatEnum::ABGR8888, width, height)
            .map_err(sdl_error)?;
        // See SdlBackend::new: the texture is renderer-owned after creation.
        let texture = unsafe {
            std::mem::transmute::<sdl2::render::Texture<'_>, sdl2::render::Texture<'static>>(
                texture,
            )
        };
        self.texture = texture;
        self.rgba = black_framebuffer(width, height)?;
        self.width = width;
        self.height = height;
        Ok(())
    }

    fn present_bgra(
        &mut self,
        src: &[u8],
        width: u32,
        height: u32,
        pitch: u32,
        bpp: u32,
    ) -> Result<()> {
        copy_bgra_to_rgba(
            &mut self.rgba,
            self.width,
            self.height,
            src,
            width,
            height,
            pitch,
            bpp,
        );
        self.present()
    }

    fn present(&mut self) -> Result<()> {
        self.texture
            .update(None, &self.rgba, self.width as usize * 4)
            .map_err(sdl_error)?;
        self.canvas
            .copy(&self.texture, None, None)
            .map_err(sdl_error)?;
        self.canvas.present();
        self.maybe_write_signal_screenshot();
        Ok(())
    }

    fn wants_event_poll(&self) -> bool {
        true
    }

    fn uses_wall_clock(&self) -> bool {
        true
    }

    fn poll_events(&mut self) -> Result<Vec<BackendEvent>> {
        use sdl2::event::Event;
        use sdl2::mouse::MouseButton;

        self.maybe_write_signal_screenshot();
        let mut events = Vec::new();
        let width = self.width;
        let height = self.height;
        for event in self.event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => events.push(BackendEvent::Quit),
                Event::MouseMotion { x, y, .. } => {
                    let (x, y) = Self::sdl_xy(width, height, x, y);
                    events.push(BackendEvent::MouseMove { x, y });
                }
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    x,
                    y,
                    ..
                } => {
                    let (x, y) = Self::sdl_xy(width, height, x, y);
                    events.push(BackendEvent::MouseButtonDown { x, y });
                }
                Event::MouseButtonUp {
                    mouse_btn: MouseButton::Left,
                    x,
                    y,
                    ..
                } => {
                    let (x, y) = Self::sdl_xy(width, height, x, y);
                    events.push(BackendEvent::MouseButtonUp { x, y });
                }
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Right,
                    x,
                    y,
                    ..
                } => {
                    let (x, y) = Self::sdl_xy(width, height, x, y);
                    events.push(BackendEvent::MouseRightButtonDown { x, y });
                }
                Event::MouseButtonUp {
                    mouse_btn: MouseButton::Right,
                    x,
                    y,
                    ..
                } => {
                    let (x, y) = Self::sdl_xy(width, height, x, y);
                    events.push(BackendEvent::MouseRightButtonUp { x, y });
                }
                Event::KeyDown {
                    keycode: Some(keycode),
                    repeat: false,
                    ..
                } => {
                    if let Some(vk) = sdl_keycode_to_vk(keycode) {
                        events.push(BackendEvent::KeyDown { vk });
                    }
                }
                Event::KeyUp {
                    keycode: Some(keycode),
                    ..
                } => {
                    if let Some(vk) = sdl_keycode_to_vk(keycode) {
                        events.push(BackendEvent::KeyUp { vk });
                    }
                }
                Event::TextInput { text, .. } => {
                    events.push(BackendEvent::TextInput { text });
                }
                _ => {}
            }
        }
        self.service_ws_commands(&mut events);
        Ok(events)
    }

    fn write_png(&self, path: &Path) -> Result<()> {
        png::write_rgba_png(path, self.width, self.height, &self.rgba)
    }
}

#[cfg(feature = "sdl2")]
fn sdl_error(err: impl std::fmt::Display) -> crate::Error {
    crate::Error::Hle(format!("sdl2: {err}"))
}

#[cfg(feature = "sdl2")]
struct SdlWsControl {
    rx: Receiver<SdlWsCommand>,
}

#[cfg(feature = "sdl2")]
enum SdlWsCommand {
    Event(BackendEvent),
    Screenshot { reply: Sender<Result<Vec<u8>>> },
}

#[cfg(feature = "sdl2")]
fn start_sdl_ws_control(addr: &str) -> Result<SdlWsControl> {
    let listener = TcpListener::bind(addr).map_err(Error::Io)?;
    let local_addr = listener.local_addr().map_err(Error::Io)?;
    let (tx, rx) = mpsc::channel();
    eprintln!("wemu: SDL websocket control listening on ws://{local_addr}");
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let tx = tx.clone();
                    thread::spawn(move || {
                        if let Err(err) = handle_sdl_ws_client(stream, tx) {
                            eprintln!("wemu: SDL websocket client error: {err}");
                        }
                    });
                }
                Err(err) => {
                    eprintln!("wemu: SDL websocket accept error: {err}");
                    break;
                }
            }
        }
    });
    Ok(SdlWsControl { rx })
}

#[cfg(feature = "sdl2")]
fn handle_sdl_ws_client(mut stream: TcpStream, tx: Sender<SdlWsCommand>) -> std::io::Result<()> {
    let key = read_ws_handshake_key(&mut stream)?;
    let accept = ws_accept_key(&key);
    write!(
        stream,
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\r\n"
    )?;

    loop {
        let Some((opcode, payload)) = read_ws_frame(&mut stream)? else {
            return Ok(());
        };
        match opcode {
            0x1 => {
                let text = String::from_utf8_lossy(&payload);
                handle_ws_text_command(&mut stream, &tx, text.trim())?;
            }
            0x8 => return Ok(()),
            0x9 => write_ws_frame(&mut stream, 0xA, &payload)?,
            _ => write_ws_frame(&mut stream, 0x1, b"err unsupported opcode\n")?,
        }
    }
}

#[cfg(feature = "sdl2")]
fn handle_ws_text_command(
    stream: &mut TcpStream,
    tx: &Sender<SdlWsCommand>,
    line: &str,
) -> std::io::Result<()> {
    let mut parts = line.split_whitespace();
    let Some(cmd) = parts.next() else {
        write_ws_frame(stream, 0x1, b"err empty command\n")?;
        return Ok(());
    };
    match cmd {
        "ping" => write_ws_frame(stream, 0x1, b"pong\n")?,
        "text" => {
            let text = parts.collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                write_ws_frame(stream, 0x1, b"err expected text\n")?;
                return Ok(());
            }
            send_ws_event(tx, BackendEvent::Text { text }, stream)?;
        }
        "keydown" | "keyup" => {
            let Some(key) = parts.next() else {
                write_ws_frame(stream, 0x1, b"err expected key\n")?;
                return Ok(());
            };
            let Some(vk) = parse_ws_key(key) else {
                write_ws_frame(stream, 0x1, b"err unknown key\n")?;
                return Ok(());
            };
            let event = if cmd == "keydown" {
                BackendEvent::KeyDown { vk }
            } else {
                BackendEvent::KeyUp { vk }
            };
            send_ws_event(tx, event, stream)?;
        }
        "key" => {
            let Some(key) = parts.next() else {
                write_ws_frame(stream, 0x1, b"err expected key\n")?;
                return Ok(());
            };
            let Some(vk) = parse_ws_key(key) else {
                write_ws_frame(stream, 0x1, b"err unknown key\n")?;
                return Ok(());
            };
            let mut ok = tx
                .send(SdlWsCommand::Event(BackendEvent::KeyDown { vk }))
                .is_ok();
            if let Some(text) = ws_text_for_key(key, vk) {
                ok = ok
                    && tx
                        .send(SdlWsCommand::Event(BackendEvent::Text { text }))
                        .is_ok();
            }
            ok = ok
                && tx
                    .send(SdlWsCommand::Event(BackendEvent::KeyUp { vk }))
                    .is_ok();
            if !ok {
                write_ws_frame(stream, 0x1, b"err emulator channel closed\n")?;
            } else {
                write_ws_frame(stream, 0x1, b"ok\n")?;
            }
        }
        "move" | "down" | "up" | "rightdown" | "rightup" => {
            let Some((x, y)) = parse_ws_xy(parts.next(), parts.next()) else {
                write_ws_frame(stream, 0x1, b"err expected x y\n")?;
                return Ok(());
            };
            let event = match cmd {
                "move" => BackendEvent::MouseMove { x, y },
                "down" => BackendEvent::MouseButtonDown { x, y },
                "up" => BackendEvent::MouseButtonUp { x, y },
                "rightdown" => BackendEvent::MouseRightButtonDown { x, y },
                _ => BackendEvent::MouseRightButtonUp { x, y },
            };
            send_ws_event(tx, event, stream)?;
        }
        "click" => {
            let Some((x, y)) = parse_ws_xy(parts.next(), parts.next()) else {
                write_ws_frame(stream, 0x1, b"err expected x y\n")?;
                return Ok(());
            };
            if tx
                .send(SdlWsCommand::Event(BackendEvent::MouseButtonDown { x, y }))
                .is_err()
                || tx
                    .send(SdlWsCommand::Event(BackendEvent::MouseButtonUp { x, y }))
                    .is_err()
            {
                write_ws_frame(stream, 0x1, b"err emulator channel closed\n")?;
            } else {
                write_ws_frame(stream, 0x1, b"ok\n")?;
            }
        }
        "rightclick" => {
            let Some((x, y)) = parse_ws_xy(parts.next(), parts.next()) else {
                write_ws_frame(stream, 0x1, b"err expected x y\n")?;
                return Ok(());
            };
            if tx
                .send(SdlWsCommand::Event(BackendEvent::MouseRightButtonDown {
                    x,
                    y,
                }))
                .is_err()
                || tx
                    .send(SdlWsCommand::Event(BackendEvent::MouseRightButtonUp {
                        x,
                        y,
                    }))
                    .is_err()
            {
                write_ws_frame(stream, 0x1, b"err emulator channel closed\n")?;
            } else {
                write_ws_frame(stream, 0x1, b"ok\n")?;
            }
        }
        "screenshot" => {
            let (reply_tx, reply_rx) = mpsc::channel();
            if tx
                .send(SdlWsCommand::Screenshot { reply: reply_tx })
                .is_err()
            {
                write_ws_frame(stream, 0x1, b"err emulator channel closed\n")?;
                return Ok(());
            }
            match reply_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(Ok(png)) => write_ws_frame(stream, 0x2, &png)?,
                Ok(Err(err)) => {
                    write_ws_frame(stream, 0x1, format!("err {err}\n").as_bytes())?;
                }
                Err(_) => write_ws_frame(stream, 0x1, b"err screenshot timeout\n")?,
            }
        }
        _ => write_ws_frame(stream, 0x1, b"err unknown command\n")?,
    }
    Ok(())
}

#[cfg(feature = "sdl2")]
fn send_ws_event(
    tx: &Sender<SdlWsCommand>,
    event: BackendEvent,
    stream: &mut TcpStream,
) -> std::io::Result<()> {
    if tx.send(SdlWsCommand::Event(event)).is_err() {
        write_ws_frame(stream, 0x1, b"err emulator channel closed\n")
    } else {
        write_ws_frame(stream, 0x1, b"ok\n")
    }
}

#[cfg(feature = "sdl2")]
fn parse_ws_xy(x: Option<&str>, y: Option<&str>) -> Option<(u32, u32)> {
    Some((x?.parse().ok()?, y?.parse().ok()?))
}

#[cfg(feature = "sdl2")]
fn parse_ws_key(key: &str) -> Option<u32> {
    crate::journal::parse_key_value(key)
}

#[cfg(feature = "sdl2")]
fn ws_text_for_key(key: &str, vk: u32) -> Option<String> {
    if key.chars().count() == 1 {
        let ch = key.chars().next()?;
        if ch.is_ascii_alphanumeric() {
            return Some(ch.to_string());
        }
    }
    match vk {
        0x20 => Some(" ".to_string()),
        0x0d => Some("\r".to_string()),
        _ => None,
    }
}

#[cfg(feature = "sdl2")]
fn sdl_keycode_to_vk(keycode: sdl2::keyboard::Keycode) -> Option<u32> {
    use sdl2::keyboard::Keycode;
    match keycode {
        Keycode::A => Some(0x41),
        Keycode::B => Some(0x42),
        Keycode::C => Some(0x43),
        Keycode::D => Some(0x44),
        Keycode::E => Some(0x45),
        Keycode::F => Some(0x46),
        Keycode::G => Some(0x47),
        Keycode::H => Some(0x48),
        Keycode::I => Some(0x49),
        Keycode::J => Some(0x4a),
        Keycode::K => Some(0x4b),
        Keycode::L => Some(0x4c),
        Keycode::M => Some(0x4d),
        Keycode::N => Some(0x4e),
        Keycode::O => Some(0x4f),
        Keycode::P => Some(0x50),
        Keycode::Q => Some(0x51),
        Keycode::R => Some(0x52),
        Keycode::S => Some(0x53),
        Keycode::T => Some(0x54),
        Keycode::U => Some(0x55),
        Keycode::V => Some(0x56),
        Keycode::W => Some(0x57),
        Keycode::X => Some(0x58),
        Keycode::Y => Some(0x59),
        Keycode::Z => Some(0x5a),
        Keycode::Num0 | Keycode::Kp0 => Some(0x30),
        Keycode::Num1 | Keycode::Kp1 => Some(0x31),
        Keycode::Num2 | Keycode::Kp2 => Some(0x32),
        Keycode::Num3 | Keycode::Kp3 => Some(0x33),
        Keycode::Num4 | Keycode::Kp4 => Some(0x34),
        Keycode::Num5 | Keycode::Kp5 => Some(0x35),
        Keycode::Num6 | Keycode::Kp6 => Some(0x36),
        Keycode::Num7 | Keycode::Kp7 => Some(0x37),
        Keycode::Num8 | Keycode::Kp8 => Some(0x38),
        Keycode::Num9 | Keycode::Kp9 => Some(0x39),
        Keycode::Semicolon => Some(0xba),
        Keycode::Equals => Some(0xbb),
        Keycode::Comma => Some(0xbc),
        Keycode::Minus => Some(0xbd),
        Keycode::Period => Some(0xbe),
        Keycode::Slash => Some(0xbf),
        Keycode::Backquote => Some(0xc0),
        Keycode::LeftBracket => Some(0xdb),
        Keycode::Backslash => Some(0xdc),
        Keycode::RightBracket => Some(0xdd),
        Keycode::Quote => Some(0xde),
        Keycode::Space => Some(0x20),
        Keycode::Return | Keycode::KpEnter => Some(0x0d),
        Keycode::KpDivide => Some(0x6f),
        Keycode::Backspace => Some(0x08),
        Keycode::Tab => Some(0x09),
        Keycode::Escape => Some(0x1b),
        Keycode::LShift | Keycode::RShift => Some(0x10),
        Keycode::LCtrl | Keycode::RCtrl => Some(0x11),
        Keycode::LAlt | Keycode::RAlt => Some(0x12),
        Keycode::Left => Some(0x25),
        Keycode::Up => Some(0x26),
        Keycode::Right => Some(0x27),
        Keycode::Down => Some(0x28),
        Keycode::Delete => Some(0x2e),
        Keycode::Home => Some(0x24),
        Keycode::End => Some(0x23),
        Keycode::F1 => Some(0x70),
        Keycode::F2 => Some(0x71),
        Keycode::F3 => Some(0x72),
        Keycode::F4 => Some(0x73),
        Keycode::F5 => Some(0x74),
        Keycode::F6 => Some(0x75),
        Keycode::F7 => Some(0x76),
        Keycode::F8 => Some(0x77),
        Keycode::F9 => Some(0x78),
        Keycode::F10 => Some(0x79),
        Keycode::F11 => Some(0x7a),
        Keycode::F12 => Some(0x7b),
        _ => None,
    }
}

#[cfg(feature = "sdl2")]
fn read_ws_handshake_key(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        header.push(byte[0]);
        if header.len() > 8192 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "websocket header too large",
            ));
        }
    }
    let text = String::from_utf8_lossy(&header);
    for line in text.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("Sec-WebSocket-Key") {
            return Ok(value.trim().to_string());
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "missing Sec-WebSocket-Key",
    ))
}

#[cfg(feature = "sdl2")]
fn read_ws_frame(stream: &mut TcpStream) -> std::io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; 2];
    match stream.read_exact(&mut head) {
        Ok(()) => {}
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
            ) =>
        {
            return Ok(None);
        }
        Err(err) => return Err(err),
    }
    let opcode = head[0] & 0x0f;
    let masked = (head[1] & 0x80) != 0;
    let mut len = (head[1] & 0x7f) as u64;
    if len == 126 {
        let mut bytes = [0u8; 2];
        stream.read_exact(&mut bytes)?;
        len = u16::from_be_bytes(bytes) as u64;
    } else if len == 127 {
        let mut bytes = [0u8; 8];
        stream.read_exact(&mut bytes)?;
        len = u64::from_be_bytes(bytes);
    }
    if len > 16 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "websocket frame too large",
        ));
    }
    let mut mask = [0u8; 4];
    if masked {
        stream.read_exact(&mut mask)?;
    }
    let mut payload = vec![0u8; len as usize];
    stream.read_exact(&mut payload)?;
    if masked {
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[i & 3];
        }
    }
    Ok(Some((opcode, payload)))
}

#[cfg(feature = "sdl2")]
fn write_ws_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut header = Vec::with_capacity(10);
    header.push(0x80 | (opcode & 0x0f));
    if payload.len() < 126 {
        header.push(payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        header.push(126);
        header.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        header.push(127);
        header.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    stream.write_all(&header)?;
    stream.write_all(payload)?;
    Ok(())
}

#[cfg(feature = "sdl2")]
fn ws_accept_key(key: &str) -> String {
    let mut bytes = key.as_bytes().to_vec();
    bytes.extend_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64_encode(&sha1_digest(&bytes))
}

#[cfg(feature = "sdl2")]
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(b2 & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(feature = "sdl2")]
fn sha1_digest(bytes: &[u8]) -> [u8; 20] {
    let mut h0 = 0x6745_2301u32;
    let mut h1 = 0xefcd_ab89u32;
    let mut h2 = 0x98ba_dcfeu32;
    let mut h3 = 0x1032_5476u32;
    let mut h4 = 0xc3d2_e1f0u32;
    let bit_len = (bytes.len() as u64) * 8;
    let mut msg = bytes.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;
        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (i, word) in [h0, h1, h2, h3, h4].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Backend, HeadlessBackend};

    #[test]
    fn present_bgra_converts_rgb565_to_rgba() {
        let mut backend = HeadlessBackend::new(2, 1);
        let src = [
            0x00, 0xf8, // red
            0xe0, 0x07, // green
        ];

        backend.present_bgra(&src, 2, 1, 4, 16).unwrap();

        assert_eq!(
            &backend.framebuffer_mut()[..8],
            &[255, 0, 0, 255, 0, 255, 0, 255]
        );
    }

    #[test]
    fn present_bgra_converts_bgr24_to_rgba() {
        let mut backend = HeadlessBackend::new(1, 1);
        let src = [3, 2, 1];

        backend.present_bgra(&src, 1, 1, 3, 24).unwrap();

        assert_eq!(&backend.framebuffer_mut()[..4], &[1, 2, 3, 255]);
    }

    #[test]
    fn headless_resize_updates_dimensions_and_clears_framebuffer() {
        let mut backend = HeadlessBackend::new(1, 1);
        backend.framebuffer_mut()[..4].copy_from_slice(&[9, 8, 7, 255]);

        backend.resize(3, 2).unwrap();

        assert_eq!(backend.width(), 3);
        assert_eq!(backend.height(), 2);
        assert_eq!(backend.framebuffer().len(), 3 * 2 * 4);
        assert!(backend
            .framebuffer()
            .chunks_exact(4)
            .all(|px| px == [0, 0, 0, 255]));
    }
}
