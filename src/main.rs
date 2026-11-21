#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]

mod capture;
mod controls;
mod overlay;
mod settings;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(target_os = "windows")]
#[path = "tray_windows.rs"]
mod tray;

#[cfg(target_os = "linux")]
use anyhow::Context as _;
use anyhow::{Result, bail};
use async_channel::Receiver;
#[cfg(target_os = "linux")]
use futures_lite::future::{block_on, or};
use gpui::{App, QuitMode};
#[cfg(target_os = "linux")]
use ksni::TrayMethods;
use std::{cell::RefCell, env, process::ExitCode, rc::Rc};
use tray::Command;
#[cfg(target_os = "linux")]
use wayland_client::globals::{GlobalListContents, registry_queue_init};
#[cfg(target_os = "linux")]
use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_registry::WlRegistry};

fn main() -> ExitCode {
    #[cfg(target_os = "windows")]
    let console = unsafe {
        windows::Win32::System::Console::AttachConsole(
            windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
        )
        .is_ok()
    };
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("saiki: {err}");
            #[cfg(target_os = "windows")]
            if !console {
                use windows::{
                    Win32::UI::WindowsAndMessaging::*,
                    core::{HSTRING, w},
                };
                unsafe {
                    let _ = MessageBoxW(
                        None,
                        &HSTRING::from(err.to_string()),
                        w!("Saiki"),
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    let quit = args.as_slice() == ["--quit"];
    let demo = match args.as_slice() {
        [] => false,
        [_] if quit => false,
        [arg] if arg == "--demo" => true,
        [arg] if arg == "--version" || arg == "-V" => {
            println!("Saiki {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        [arg] if arg == "--help" || arg == "-h" => {
            println!(
                "Saiki shows keypresses on Wayland and Windows.\n\nUsage: saiki [--demo | --quit | --help | --version]\n\n  --demo  Show a sample stack without opening keyboard devices.\n  --quit  Quit the running Saiki instance."
            );
            return Ok(());
        }
        _ => bail!("unknown arguments. Run saiki --help."),
    };

    let (commands, incoming) = async_channel::unbounded();
    #[cfg(target_os = "linux")]
    let session_bus = if demo {
        None
    } else {
        let bus = block_on(zbus::Connection::session()).context("connect to the session bus")?;
        if quit {
            return block_on(tray::forward(&bus, "Quit"))
                .context("quit the running Saiki instance");
        }
        match block_on(tray::register(&bus, commands.clone())) {
            Ok(()) => Some(bus),
            Err(zbus::Error::NameTaken) => {
                return block_on(tray::forward(&bus, "Show"))
                    .context("open the running Saiki instance");
            }
            Err(error) => return Err(error).context("register Saiki on the session bus"),
        }
    };

    #[cfg(target_os = "windows")]
    let session = if demo {
        None
    } else {
        match tray::Session::open(commands.clone(), quit)? {
            Some(session) => Some(session),
            None => return Ok(()),
        }
    };
    let layer_shell = overlay_available()?;
    if demo && !layer_shell {
        bail!("{}", capture::CaptureError::Overlay.message());
    }
    #[cfg(target_os = "linux")]
    let session = session_bus;

    let result = Rc::new(RefCell::new(Ok(())));
    let startup = result.clone();
    gpui_platform::application().run(move |cx: &mut App| {
        let opened = if let Some(session) = session {
            start_session(cx, layer_shell, session, commands, incoming)
        } else {
            cx.set_quit_mode(QuitMode::LastWindowClosed);
            overlay::show(
                cx,
                &mut None,
                settings::Position::load(),
                settings::FadeDelay::load().duration(),
                overlay::demo(),
            )
        };
        if let Err(error) = opened {
            *startup.borrow_mut() = Err(error);
            cx.quit();
        }
    });
    result.replace(Ok(()))
}

#[cfg(target_os = "linux")]
fn start_session(
    cx: &mut App,
    layer_shell: bool,
    bus: zbus::Connection,
    commands: async_channel::Sender<Command>,
    incoming: Receiver<Command>,
) -> Result<()> {
    cx.set_quit_mode(QuitMode::Explicit);
    let (status_sender, statuses) = async_channel::bounded(1);
    let state = controls::Controls::new(cx, layer_shell, status_sender);
    let mut controls = controls::Controls::show(cx, &state)?;
    let text = |message| state.read(cx).text(message);
    let indicator = tray::Indicator {
        commands,
        status_text: text("Capture paused"),
        capturing: false,
        labels: tray::Labels {
            show: text("Open Saiki"),
            start: text("Start capture"),
            pause: text("Pause capture"),
            quit: text("Quit Saiki"),
        },
    };
    cx.spawn(async move |cx| {
        // The session owns the controls entity and bus registration even with no open windows.
        let _bus = bus;
        let tray = indicator.assume_sni_available(true).spawn().await;
        if let Err(error) = &tray {
            eprintln!("saiki: tray unavailable: {error}");
        }
        enum Event {
            Input(Command),
            Status(&'static str),
        }
        while let Some(event) = or(
            async { incoming.recv().await.ok().map(Event::Input) },
            async { statuses.recv().await.ok().map(Event::Status) },
        )
        .await
        {
            match event {
                Event::Input(Command::Show) => cx.update(|cx| {
                    if controls
                        .update(cx, |_, window, _| window.activate_window())
                        .is_err()
                    {
                        match controls::Controls::show(cx, &state) {
                            Ok(window) => controls = window,
                            Err(error) => eprintln!("saiki: open controls: {error}"),
                        }
                    }
                }),
                Event::Input(Command::Start) => cx.update(|cx| {
                    state.update(cx, |state, cx| state.start(cx));
                }),
                Event::Input(Command::Pause) => cx.update(|cx| {
                    state.update(cx, |state, cx| state.stop(cx));
                }),
                Event::Input(Command::Quit) => {
                    cx.update(|cx| cx.quit());
                    break;
                }
                Event::Status(status) => {
                    if let Ok(handle) = &tray {
                        let localized = cx.update(|cx| state.read(cx).text(status));
                        handle
                            .update(move |indicator| {
                                indicator.capturing =
                                    matches!(status, "Capturing keys" | "Opening keyboard…");
                                indicator.status_text = localized;
                            })
                            .await;
                    }
                }
            }
        }
    })
    .detach();
    Ok(())
}

#[cfg(target_os = "linux")]
struct Registry;

#[cfg(target_os = "linux")]
impl Dispatch<WlRegistry, GlobalListContents> for Registry {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(target_os = "linux")]
fn overlay_available() -> Result<bool> {
    let connection = Connection::connect_to_env()
        .map_err(|err| anyhow::anyhow!("connect to a Wayland session: {err}"))?;
    let (globals, _) = registry_queue_init::<Registry>(&connection)?;
    Ok(globals.contents().with_list(|globals| {
        globals
            .iter()
            .any(|global| global.interface == "zwlr_layer_shell_v1")
    }))
}

#[cfg(target_os = "windows")]
fn overlay_available() -> Result<bool> {
    Ok(true)
}

#[cfg(target_os = "windows")]
fn start_session(
    cx: &mut App,
    available: bool,
    session: tray::Session,
    _commands: async_channel::Sender<Command>,
    incoming: Receiver<Command>,
) -> Result<()> {
    cx.set_quit_mode(QuitMode::Explicit);
    let (status_sender, statuses) = async_channel::bounded(1);
    let state = controls::Controls::new(cx, available, status_sender);
    let mut controls = controls::Controls::show(cx, &state)?;
    cx.spawn(async move |cx| {
        let _session = session;
        while let Some(command) = next_windows_command(&incoming, &statuses).await {
            match command {
                Command::Show => cx.update(|cx| {
                    if controls
                        .update(cx, |_, window, _| window.activate_window())
                        .is_err()
                    {
                        match controls::Controls::show(cx, &state) {
                            Ok(window) => controls = window,
                            Err(error) => eprintln!("saiki: open controls: {error}"),
                        }
                    }
                }),
                Command::Start => cx.update(|cx| state.update(cx, |state, cx| state.start(cx))),
                Command::Pause => cx.update(|cx| state.update(cx, |state, cx| state.stop(cx))),
                Command::Quit => {
                    cx.update(|cx| cx.quit());
                    break;
                }
            }
        }
    })
    .detach();
    Ok(())
}

// Status notifications are drained separately from command-channel closure. Once
// statuses close, leave that branch pending so it cannot cause a busy loop.
#[cfg(any(target_os = "windows", test))]
async fn next_windows_command(
    incoming: &Receiver<Command>,
    statuses: &Receiver<&'static str>,
) -> Option<Command> {
    enum Event {
        Input(Option<Command>),
        Status,
    }
    loop {
        let event =
            futures_lite::future::or(async { Event::Input(incoming.recv().await.ok()) }, async {
                match statuses.recv().await {
                    Ok(_) => Event::Status,
                    Err(_) => std::future::pending().await,
                }
            })
            .await;
        if let Event::Input(command) = event {
            return command;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_commands_end_the_session_after_status_updates() {
        futures_lite::future::block_on(async {
            let (commands, incoming) = async_channel::unbounded();
            let (status, statuses) = async_channel::unbounded();
            status.try_send("Capture paused").unwrap();
            let mut next = std::pin::pin!(next_windows_command(&incoming, &statuses));
            // With commands still open, consuming a status must leave the session waiting.
            assert!(
                futures_lite::future::poll_once(next.as_mut())
                    .await
                    .is_none()
            );
            assert!(statuses.is_empty());
            drop(commands);
            assert!(next.await.is_none());
        });
    }

    #[test]
    fn closed_statuses_do_not_hide_commands_or_command_closure() {
        futures_lite::future::block_on(async {
            let (commands, incoming) = async_channel::unbounded();
            let (status, statuses) = async_channel::unbounded();
            drop(status);
            commands.try_send(Command::Show).unwrap();
            assert!(matches!(
                next_windows_command(&incoming, &statuses).await,
                Some(Command::Show)
            ));
            drop(commands);
            assert!(next_windows_command(&incoming, &statuses).await.is_none());
        });
    }
}
