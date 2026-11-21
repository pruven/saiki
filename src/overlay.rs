use crate::capture::Stroke;
use crate::position::Position;
use gpui::layer_shell::{KeyboardInteractivity, Layer, LayerShellOptions};
use gpui::prelude::*;
use gpui::{
    App, Bounds, BoxShadow, Context, Div, FontWeight, Task, Window, WindowBackgroundAppearance,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, div, point, px, rgb, rgba, size,
};
use std::time::{Duration, Instant};

const CARD_WIDTH: f32 = 340.;
const CARD_HEIGHT: f32 = 82.;
const CARD_GAP: f32 = 9.;
const GUTTER: f32 = 18.;
const SURFACE_WIDTH: f32 = CARD_WIDTH + GUTTER + 44.;
const MAX_CARDS: usize = 4;
const SURFACE_HEIGHT: f32 = GUTTER + MAX_CARDS as f32 * (CARD_HEIGHT + CARD_GAP) + 34.;
const LIFETIME: Duration = Duration::from_millis(2600);
const ENTER: Duration = Duration::from_millis(360);
const EXIT: Duration = Duration::from_millis(180);

pub const TEXT: u32 = 0xf7f7f7;
pub const MUTED: u32 = 0xb1b1b5;
pub const SURFACE: u32 = 0x262626;
pub const EDGE: u32 = 0xffffff17;

#[derive(Debug, PartialEq, Eq)]
enum Content {
    Word { text: String, open: bool },
    Key(Vec<String>),
}

struct Card {
    content: Content,
    born: Instant,
    last_key: Instant,
    from: f32,
    row: f32,
    moved: Instant,
}

impl Card {
    fn y(&self, now: Instant) -> f32 {
        let t = progress(now - self.moved, ENTER);
        let spring = 1. + 2.2 * (t - 1.).powi(3) + 1.2 * (t - 1.).powi(2);
        self.from + (self.row - self.from) * spring
    }

    fn fade_at(&self) -> Instant {
        self.last_key + LIFETIME
    }

    fn expires(&self) -> Instant {
        self.fade_at() + EXIT
    }
}

pub struct Overlay {
    position: Position,
    cards: Vec<Card>,
    wake_at: Option<Instant>,
    wake: Option<Task<()>>,
}

impl Overlay {
    fn new(position: Position) -> Self {
        Self {
            position,
            cards: Vec::new(),
            wake_at: None,
            wake: None,
        }
    }

    fn push(&mut self, stroke: Stroke, now: Instant) {
        self.cards.retain(|card| card.expires() > now);
        if let Some(card) = self.cards.first_mut()
            && now < card.fade_at()
            && let Content::Word { text, open } = &mut card.content
            && *open
        {
            match stroke {
                Stroke::Text(character) => {
                    text.push(character);
                    // Keep only the tail when a word is too long to display.
                    if text.len() > 1024 {
                        text.drain(..text.chars().next().unwrap().len_utf8());
                    }
                    card.last_key = now;
                    return;
                }
                Stroke::Backspace => {
                    text.pop();
                    card.last_key = now;
                    if text.is_empty() {
                        self.cards.remove(0);
                        self.restack(now);
                    }
                    return;
                }
                Stroke::Space => {
                    *open = false;
                    card.last_key = now;
                    return;
                }
                Stroke::Key { .. } => *open = false,
            }
        }

        let content = match stroke {
            Stroke::Text(character) => Content::Word {
                text: character.to_string(),
                open: true,
            },
            Stroke::Space => Content::Key(vec!["Space".into()]),
            Stroke::Backspace => Content::Key(vec!["Backspace".into()]),
            Stroke::Key { caps, .. } => Content::Key(caps),
        };
        self.cards.truncate(MAX_CARDS - 1);
        self.cards.insert(
            0,
            Card {
                content,
                born: now,
                last_key: now,
                from: 0.,
                row: 0.,
                moved: now,
            },
        );
        self.restack(now);
    }

    fn restack(&mut self, now: Instant) {
        for (index, card) in self.cards.iter_mut().enumerate() {
            let row = index as f32 * (CARD_HEIGHT + CARD_GAP);
            if card.row != row {
                card.from = card.y(now);
                card.row = row;
                card.moved = now;
            }
        }
    }
}

impl Render for Overlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        self.cards.retain(|card| card.expires() > now);
        self.restack(now);
        if self
            .cards
            .iter()
            .any(|card| now - card.moved < ENTER || now >= card.fade_at())
        {
            window.request_animation_frame();
        }
        if self.cards.is_empty() {
            window.remove_window();
        }

        let next = self
            .cards
            .iter()
            .map(Card::fade_at)
            .filter(|deadline| *deadline > now)
            .min();
        if next != self.wake_at {
            // GPUI only needs frames during movement. Wake it when the fade should start.
            self.wake_at = next;
            self.wake = next.map(|deadline| {
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(deadline.saturating_duration_since(Instant::now()))
                        .await;
                    let _ = this.update(cx, |_, cx| cx.notify());
                })
            });
        }

        let horizontal = self.position.horizontal();
        let left = match horizontal {
            -1 => GUTTER,
            1 => SURFACE_WIDTH - CARD_WIDTH - GUTTER,
            _ => (SURFACE_WIDTH - CARD_WIDTH) / 2.,
        };
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .children(self.cards.iter().rev().map(|card| {
                let enter = progress(now - card.born, ENTER);
                let exit = progress(now.saturating_duration_since(card.fade_at()), EXIT);
                let slide = 36. * (1. - enter).powi(3) + 24. * exit.powi(2);
                let vertical_slide = if horizontal == 0 { slide / 2. } else { 0. };
                let offset = GUTTER + card.y(now) - vertical_slide;
                let y = if self.position.is_bottom() {
                    SURFACE_HEIGHT - CARD_HEIGHT - offset
                } else {
                    offset
                };
                div()
                    .absolute()
                    .left(px(left + f32::from(horizontal) * slide))
                    .top(px(y))
                    .opacity((enter * 2.7).min(1.) * (1. - exit))
                    .child(card_view(&card.content))
            }))
    }
}

fn progress(elapsed: Duration, duration: Duration) -> f32 {
    (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0., 1.)
}

pub fn show(
    cx: &mut App,
    overlay: &mut Option<WindowHandle<Overlay>>,
    position: Position,
    strokes: impl IntoIterator<Item = Stroke>,
) -> anyhow::Result<()> {
    let mut strokes = strokes.into_iter().peekable();
    if strokes.peek().is_none() {
        return Ok(());
    }
    let populate = |view: &mut Overlay| {
        let now = Instant::now();
        for stroke in strokes {
            view.push(stroke, now);
        }
    };
    if let Some(handle) = overlay
        && handle.read(cx).is_ok()
    {
        return handle.update(cx, |view, _, cx| {
            populate(view);
            cx.notify();
        });
    }
    *overlay = Some(open(cx, position, populate)?);
    Ok(())
}

pub fn relocate(
    cx: &mut App,
    overlay: &mut Option<WindowHandle<Overlay>>,
    position: Position,
) -> anyhow::Result<()> {
    let Some(handle) = overlay.take() else {
        return Ok(());
    };
    let Ok(cards) = handle.update(cx, |view, window, _| {
        window.remove_window();
        std::mem::take(&mut view.cards)
    }) else {
        return Ok(());
    };
    if cards.iter().any(|card| card.expires() > Instant::now()) {
        *overlay = Some(open(cx, position, |view| view.cards = cards)?);
    }
    Ok(())
}

fn open(
    cx: &mut App,
    position: Position,
    populate: impl FnOnce(&mut Overlay),
) -> anyhow::Result<WindowHandle<Overlay>> {
    cx.open_window(
        WindowOptions {
            titlebar: None,
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size(px(SURFACE_WIDTH), px(SURFACE_HEIGHT)),
            ))),
            kind: WindowKind::LayerShell(LayerShellOptions {
                namespace: "saiki".into(),
                layer: Layer::Overlay,
                anchor: position.anchor(),
                keyboard_interactivity: KeyboardInteractivity::None,
                ..Default::default()
            }),
            focus: false,
            app_id: Some("saiki".into()),
            window_background: WindowBackgroundAppearance::Transparent,
            inactive_frame_interval: None,
            ..Default::default()
        },
        |window, cx| {
            window.set_input_region(Some(&[]));
            cx.new(|_| {
                let mut view = Overlay::new(position);
                populate(&mut view);
                view
            })
        },
    )
}

pub fn demo() -> impl IntoIterator<Item = Stroke> {
    [
        Stroke::Key {
            caps: vec!["Ctrl".into(), "Shift".into(), "P".into()],
            shortcut: true,
        },
        Stroke::Key {
            caps: vec!["Super".into(), "Space".into()],
            shortcut: true,
        },
    ]
    .into_iter()
    .chain("hello".chars().map(Stroke::Text))
    .chain([Stroke::Space])
}

pub fn keyboard_mark() -> Div {
    let row = || {
        div()
            .flex()
            .gap(px(1.4))
            .children((0..3).map(|_| div().size(px(1.55)).rounded(px(0.4)).bg(rgb(0xd8dcf0))))
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(20.))
        .rounded(px(7.))
        .bg(rgb(0x424651))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .w(px(12.))
                .h(px(9.))
                .gap(px(1.1))
                .border_1()
                .border_color(rgb(0xd8dcf0))
                .rounded(px(2.))
                .child(row())
                .child(div().w(px(5.)).h(px(1.)).rounded(px(0.5)).bg(rgb(0xd8dcf0))),
        )
}

fn card_view(content: &Content) -> Div {
    let keys = match content {
        Content::Word { text, .. } => div()
            .text_color(rgb(TEXT))
            .text_size(px(17.))
            .font_weight(FontWeight::SEMIBOLD)
            .line_height(px(26.))
            .text_ellipsis_start()
            .child(text.clone()),
        Content::Key(caps) => {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .children(caps.iter().cloned().map(|cap| {
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px(26.))
                        .min_w(px(26.))
                        .max_w(px(115.))
                        .px(px(8.))
                        .rounded(px(7.))
                        .bg(rgb(0x363638))
                        .border_1()
                        .border_color(rgba(0xffffff12))
                        .text_color(rgb(TEXT))
                        .font_weight(FontWeight::MEDIUM)
                        .text_size(px(13.))
                        .child(div().text_ellipsis().child(cap))
                }))
        }
    };
    div()
        .w(px(CARD_WIDTH))
        .h(px(CARD_HEIGHT))
        .p(px(12.))
        .flex()
        .flex_col()
        .justify_between()
        .rounded(px(16.))
        .bg(rgb(SURFACE))
        .border_1()
        .border_color(rgba(EDGE))
        .shadow(vec![
            BoxShadow::new(px(0.), px(2.), rgba(0x00000045).into()).blur_radius(px(6.)),
            BoxShadow::new(px(0.), px(8.), rgba(0x00000050).into()).blur_radius(px(24.)),
        ])
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(MUTED))
                .child(keyboard_mark())
                .child("Saiki"),
        )
        .child(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_letters_and_keeps_shortcuts_separate() {
        let mut overlay = Overlay::new(Position::TopRight);
        let now = Instant::now();
        for character in "hello world".chars() {
            overlay.push(
                if character == ' ' {
                    Stroke::Space
                } else {
                    Stroke::Text(character)
                },
                now,
            );
        }
        overlay.push(
            Stroke::Key {
                caps: vec!["Ctrl".into(), "C".into()],
                shortcut: true,
            },
            now,
        );
        assert_eq!(overlay.cards.len(), 3);
        assert_eq!(
            overlay.cards[2].content,
            Content::Word {
                text: "hello".into(),
                open: false
            }
        );
        assert_eq!(
            overlay.cards[1].content,
            Content::Word {
                text: "world".into(),
                open: false
            }
        );
        assert_eq!(
            overlay.cards[0].content,
            Content::Key(vec!["Ctrl".into(), "C".into()])
        );
    }

    #[test]
    fn edits_the_current_word_and_refreshes_its_timeout() {
        let mut overlay = Overlay::new(Position::TopRight);
        let now = Instant::now();
        overlay.push(Stroke::Text('h'), now);
        let later = now + Duration::from_millis(1200);
        overlay.push(Stroke::Text('é'), later);
        assert_eq!(overlay.cards[0].fade_at(), later + LIFETIME);
        overlay.push(Stroke::Backspace, later);
        assert_eq!(
            overlay.cards[0].content,
            Content::Word {
                text: "h".into(),
                open: true
            }
        );
        overlay.push(Stroke::Backspace, later);
        assert!(overlay.cards.is_empty());
    }

    #[test]
    fn keeps_the_four_latest_cards() {
        let mut overlay = Overlay::new(Position::TopRight);
        let now = Instant::now();
        for n in 0..8 {
            overlay.push(
                Stroke::Key {
                    caps: vec![n.to_string()],
                    shortcut: false,
                },
                now + Duration::from_millis(20 * n),
            );
        }
        let shown: Vec<_> = overlay
            .cards
            .iter()
            .map(|card| match &card.content {
                Content::Key(caps) => caps[0].as_str(),
                Content::Word { .. } => panic!("expected keycap"),
            })
            .collect();
        assert_eq!(shown, ["7", "6", "5", "4"]);
        assert_eq!(overlay.cards[3].row, 3. * (CARD_HEIGHT + CARD_GAP));
        assert!(overlay.cards[0].expires() > overlay.cards[3].expires());
    }
}
