use anyhow::{Result, bail};
use async_channel::Sender;
use std::{thread, time::Duration};
use windows::{
    Win32::{
        Foundation::*,
        System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::w,
};

#[derive(Clone, Copy)]
pub enum Command {
    Show,
    Start,
    Pause,
    Quit,
}

const TRAY_MESSAGE: u32 = WM_APP + 1;
const COMMAND_MESSAGE: u32 = WM_APP + 2;

pub struct Session {
    hwnd: usize,
    mutex: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl Session {
    // None means the command was forwarded to the existing process.
    pub fn open(commands: Sender<Command>, quit: bool) -> Result<Option<Self>> {
        unsafe {
            let mutex = CreateMutexW(None, false, w!("Local\\Saiki.Session"))?;
            let existing = GetLastError() == ERROR_ALREADY_EXISTS;
            if existing || quit {
                let _ = CloseHandle(mutex);
                for _ in 0..40 {
                    if let Ok(hwnd) = FindWindowW(w!("Saiki.Session"), None) {
                        let mut process = 0;
                        GetWindowThreadProcessId(hwnd, Some(&mut process));
                        let _ = AllowSetForegroundWindow(process);
                        PostMessageW(
                            Some(hwnd),
                            COMMAND_MESSAGE,
                            WPARAM(if quit { 4 } else { 1 }),
                            LPARAM(0),
                        )?;
                        return Ok(None);
                    }
                    if !existing {
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                bail!("no running Saiki instance is available");
            }
            let (ready, started) = std::sync::mpsc::sync_channel(1);
            let thread = thread::spawn(move || {
                let result = run(commands, ready.clone());
                if let Err(error) = result {
                    let _ = ready.send(Err(error.to_string()));
                }
            });
            let startup = match started.recv() {
                Ok(startup) => startup,
                Err(error) => {
                    let _ = thread.join();
                    let _ = CloseHandle(mutex);
                    bail!("start Windows tray: {error}");
                }
            };
            match startup {
                Ok(hwnd) => Ok(Some(Self {
                    hwnd,
                    mutex: mutex.0 as usize,
                    thread: Some(thread),
                })),
                Err(error) => {
                    let _ = thread.join();
                    let _ = CloseHandle(mutex);
                    bail!("start Windows tray: {error}");
                }
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            );
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            let _ = CloseHandle(HANDLE(self.mutex as *mut _));
        }
    }
}

struct State {
    commands: Sender<Command>,
    taskbar_created: u32,
}

fn run(
    commands: Sender<Command>,
    ready: std::sync::mpsc::SyncSender<Result<usize, String>>,
) -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("Saiki.Session"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            bail!(
                "register tray window: {}",
                windows::core::Error::from_win32()
            );
        }
        let mut state = Box::new(State {
            commands,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
        });
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("Saiki.Session"),
            w!("Saiki"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            Some((&mut *state as *mut State).cast()),
        )?;
        if !Shell_NotifyIconW(NIM_ADD, &icon(hwnd)).as_bool() {
            let _ = DestroyWindow(hwnd);
            bail!("add notification area icon");
        }
        let _ = ready.send(Ok(hwnd.0 as usize));
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        // Also destroy the window if the message loop ended through an error or WM_QUIT.
        // Its callbacks must stop before the state they reference is released.
        let _ = Shell_NotifyIconW(NIM_DELETE, &icon(hwnd));
        let _ = DestroyWindow(hwnd);
        drop(state);
    }
    Ok(())
}

unsafe fn icon(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: TRAY_MESSAGE,
        hIcon: unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() },
        ..Default::default()
    };
    let tip: Vec<_> = "Saiki — click to open controls".encode_utf16().collect();
    data.szTip[..tip.len()].copy_from_slice(&tip);
    data
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let state = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State).as_ref();
        if let Some(state) = state {
            let command = match message {
                COMMAND_MESSAGE | WM_COMMAND => match wparam.0 & 0xffff {
                    1 => Some(Command::Show),
                    2 => Some(Command::Start),
                    3 => Some(Command::Pause),
                    4 => Some(Command::Quit),
                    _ => None,
                },
                TRAY_MESSAGE if lparam.0 as u32 == WM_LBUTTONUP => Some(Command::Show),
                _ => None,
            };
            if let Some(command) = command {
                let _ = state.commands.try_send(command);
                return LRESULT(0);
            }
            if message == state.taskbar_created && state.taskbar_created != 0 {
                let _ = Shell_NotifyIconW(NIM_ADD, &icon(hwnd));
                return LRESULT(0);
            }
            if message == TRAY_MESSAGE && lparam.0 as u32 == WM_RBUTTONUP {
                if let Ok(menu) = CreatePopupMenu() {
                    let _ = AppendMenuW(menu, MF_STRING, 1, w!("Open Saiki"));
                    let _ = AppendMenuW(menu, MF_STRING, 2, w!("Start capture"));
                    let _ = AppendMenuW(menu, MF_STRING, 3, w!("Pause capture"));
                    let _ = AppendMenuW(menu, MF_STRING, 4, w!("Quit Saiki"));
                    let mut point = POINT::default();
                    let _ = GetCursorPos(&mut point);
                    let _ = SetForegroundWindow(hwnd);
                    let _ =
                        TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, None, hwnd, None);
                    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                    let _ = DestroyMenu(menu);
                }
                return LRESULT(0);
            }
        }
        match message {
            WM_CLOSE => {
                let _ = Shell_NotifyIconW(NIM_DELETE, &icon(hwnd));
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
