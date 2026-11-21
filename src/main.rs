mod capture;
mod controls;
mod overlay;
mod position;
mod tray;

use anyhow::{Context as _, Result, bail};
use async_channel::Receiver;
use futures_lite::future::{block_on, or};
use gpui::{App, QuitMode};
use ksni::TrayMethods;
use std::{cell::RefCell, env, process::ExitCode, rc::Rc};
use tray::Command;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_registry::WlRegistry};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("saiki: {err}");
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
                "Saiki shows keypresses on Wayland.\n\nUsage: saiki [--demo | --quit | --help | --version]\n\n  --demo  Show a sample stack without opening keyboard devices.\n  --quit  Quit the running Saiki instance."
            );
            return Ok(());
        }
        _ => bail!("unknown arguments. Run saiki --help."),
    };

    let (commands, incoming) = async_channel::unbounded();
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

    let connection = Connection::connect_to_env()
        .map_err(|err| anyhow::anyhow!("connect to a Wayland session: {err}"))?;
    let (globals, _) = registry_queue_init::<Registry>(&connection)?;
    let layer_shell = globals.contents().with_list(|globals| {
        globals
            .iter()
            .any(|global| global.interface == "zwlr_layer_shell_v1")
    });
    drop(connection);
    if demo && !layer_shell {
        bail!("{}", capture::CaptureError::Overlay.message());
    }

    let result = Rc::new(RefCell::new(Ok(())));
    let startup = result.clone();
    gpui_platform::application().run(move |cx: &mut App| {
        let opened = if let Some(bus) = session_bus {
            start_session(cx, layer_shell, bus, commands, incoming)
        } else {
            cx.set_quit_mode(QuitMode::LastWindowClosed);
            overlay::show(cx, &mut None, position::Position::load(), overlay::demo())
        };
        if let Err(error) = opened {
            *startup.borrow_mut() = Err(error);
            cx.quit();
        }
    });
    result.replace(Ok(()))
}

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

struct Registry;

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
