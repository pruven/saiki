use async_channel::Sender;
use ksni::menu::StandardItem;
use ksni::{MenuItem, ToolTip, Tray};
use zbus::{Connection, Proxy, fdo};

pub const BUS_NAME: &str = "io.github.pruven.Saiki";
const PATH: &str = "/io/github/pruven/Saiki";

#[derive(Clone, Copy)]
pub enum Command {
    Show,
    Start,
    Pause,
    Quit,
}

pub struct Service(pub Sender<Command>);

#[zbus::interface(name = "io.github.pruven.Saiki")]
impl Service {
    fn show(&self) -> fdo::Result<()> {
        self.send(Command::Show)
    }

    fn quit(&self) -> fdo::Result<()> {
        self.send(Command::Quit)
    }
}

impl Service {
    fn send(&self, command: Command) -> fdo::Result<()> {
        self.0
            .try_send(command)
            .map_err(|_| fdo::Error::Failed("Saiki is shutting down".into()))
    }
}

pub async fn forward(connection: &Connection, method: &str) -> zbus::Result<()> {
    Proxy::new(connection, BUS_NAME, PATH, BUS_NAME)
        .await?
        .call(method, &())
        .await
}

pub async fn register(connection: &Connection, sender: Sender<Command>) -> zbus::Result<()> {
    connection.object_server().at(PATH, Service(sender)).await?;
    connection
        .request_name_with_flags(BUS_NAME, fdo::RequestNameFlags::DoNotQueue.into())
        .await?;
    Ok(())
}

pub struct Indicator {
    pub commands: Sender<Command>,
    pub status_text: String,
    pub capturing: bool,
    pub labels: Labels,
}

pub struct Labels {
    pub show: String,
    pub start: String,
    pub pause: String,
    pub quit: String,
}

impl Tray for Indicator {
    fn id(&self) -> String {
        "saiki".into()
    }

    fn title(&self) -> String {
        "Saiki".into()
    }

    fn icon_name(&self) -> String {
        "input-keyboard".into()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Saiki".into(),
            description: self.status_text.clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.commands.try_send(Command::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let (label, command) = if self.capturing {
            (&self.labels.pause, Command::Pause)
        } else {
            (&self.labels.start, Command::Start)
        };
        [
            (&self.labels.show, Command::Show),
            (label, command),
            (&self.labels.quit, Command::Quit),
        ]
        .into_iter()
        .map(|(label, command)| {
            StandardItem {
                label: label.clone(),
                activate: Box::new(move |tray: &mut Self| {
                    let _ = tray.commands.try_send(command);
                }),
                ..Default::default()
            }
            .into()
        })
        .collect()
    }
}
