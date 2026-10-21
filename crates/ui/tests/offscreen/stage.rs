//! The application's window without a display: the real shell on the
//! toolkit's test platform, driven by keys, clicks and actions, waited on
//! until it is at rest, and drawn into an image.
//!
//! Text is drawn with the two fonts beside this file and nothing else:
//! the text system is given no font of the machine. The application
//! bundles no font of its own (it asks for the system's), so the pictures
//! show its layout with these two, not with what a user's machine has.
//! On macOS, where this runs only when asked ([`ON_MACOS`]), the text
//! system is the system's own (Core Text), given the same two fonts; it
//! has the machine's fonts as well, which only what the two lack falls
//! back on.
//! Icons come from what the application bundles (`app::Assets`).

use std::{
    borrow::Cow,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use bitbraid_ui::{
    prefs::{Prefs, ThemeChoice},
    session,
    shell::{Shell, ShellProbe},
    views::{
        graph::GraphProbe,
        workspace::{Workspace, WorkspaceProbe},
    },
};
use gpui_kit::{
    Action, AnyWindowHandle, App, AppContext as _, Bounds, ElementId, Entity, Focusable as _,
    HeadlessAppContext, InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Render, Window, WindowBounds, WindowOptions, component::Theme, point, px, size,
    test::TestWindowExt as _,
};
use image::RgbaImage;

/// The font of the interface, for everything that asks for the system's.
const UI_FONT: &str = "Inter";
/// The font of code, diffs and ids.
const MONO_FONT: &str = "Fira Mono";

/// Set to run the tests and draw the views on macOS (`offscreen.rs`).
#[cfg(target_os = "macos")]
pub const ON_MACOS: &str = "BITBRAID_OFFSCREEN_ON_MACOS";

/// Set to run the tests and draw the views on Windows (`offscreen.rs`).
#[cfg(windows)]
pub const ON_WINDOWS: &str = "BITBRAID_OFFSCREEN_ON_WINDOWS";

const INTER: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
const FIRA_MONO: &[u8] = include_bytes!("../fonts/FiraMono-Medium.ttf");

/// The window, in points. The frame has twice as many pixels each way
/// (the test platform draws at two to the point).
pub const WIDTH: f32 = 1280.0;
pub const HEIGHT: f32 = 720.0;

/// How long anything is waited for before the test fails saying what it
/// waited for. Far longer than it takes, also on a machine that is busy.
const PATIENCE: Duration = Duration::from_secs(60);

/// Longer than what the toolkit puts off before it draws what it was
/// given, and shorter than anything that comes and goes by itself (a
/// notification stays for seconds). An editor colours its text at once
/// only while that takes it under two milliseconds on the real clock; on
/// a busy machine it does not, and the colours come 150 ms later on the
/// scheduler's clock, which stands still here until it is moved
/// ([`Stage::pass`]). A picture of an editor is taken after this long.
pub const PUT_OFF: Duration = Duration::from_millis(200);

/// The renderer frames are drawn with: the toolkit's for this system,
/// and on Windows, which has none off screen, the renderer the toolkit
/// draws with on Linux.
fn headless_renderer() -> gpui_kit::Result<Option<Box<dyn gpui_kit::PlatformHeadlessRenderer>>> {
    #[cfg(windows)]
    {
        gpui_wgpu::WgpuHeadlessRenderer::new()
            .map(|renderer| Some(Box::new(renderer) as Box<dyn gpui_kit::PlatformHeadlessRenderer>))
    }
    #[cfg(not(windows))]
    gpui_kit::platform::current_headless_renderer()
}

/// Why there are no pictures here, when a frame cannot be drawn.
pub fn no_renderer() -> Option<String> {
    match headless_renderer() {
        Ok(Some(_)) => None,
        Ok(None) => Some("the toolkit has no headless renderer on this platform".to_owned()),
        Err(err) => Some(format!("no graphics adapter can be used ({err:#})")),
    }
}

/// The text system the views are laid out and drawn with: on Linux (and
/// on Windows, asked for) the toolkit's own, which is given no font of the
/// machine.
#[cfg(any(target_os = "linux", windows))]
fn text_system() -> Arc<dyn gpui_kit::PlatformTextSystem> {
    Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(
        UI_FONT,
    ))
}

/// On macOS the system's, Core Text, which the application draws with
/// there.
#[cfg(target_os = "macos")]
fn text_system() -> Arc<dyn gpui_kit::PlatformTextSystem> {
    gpui_kit::platform::current_platform(true).text_system()
}

/// The toolkit says which adapter it chose in its log and nowhere else.
struct AdapterLog;

static ADAPTER: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

impl log::Log for AdapterLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        let line = record.args().to_string();
        if let Some(adapter) = line.strip_prefix("Selected GPU adapter: ")
            && let Ok(mut known) = ADAPTER.lock()
        {
            *known = Some(adapter.to_owned());
        }
    }

    fn flush(&self) {}
}

/// Start listening for the adapter's name; before the first frame.
pub fn listen_for_adapter() {
    static LOG: AdapterLog = AdapterLog;
    if log::set_logger(&LOG).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// The graphics adapter the frames were drawn with, as the toolkit names
/// it; empty when it never said.
pub fn adapter() -> String {
    ADAPTER
        .lock()
        .ok()
        .and_then(|known| known.clone())
        .unwrap_or_default()
}

/// What the application is started with.
#[derive(Debug, Clone, Default)]
pub struct Start {
    /// The path on its command line.
    pub open: Option<PathBuf>,
    /// The link on its command line.
    pub link: Option<String>,
    /// `preferences.toml`, as text.
    pub preferences: String,
    /// `state.toml`, as text.
    pub state: String,
    /// Whether frames are drawn into images: it takes a graphics adapter.
    /// Without, the views are still laid out and painted into a scene.
    pub pictures: bool,
    /// Other files of the configuration directory, by name, as text.
    pub files: Vec<(String, String)>,
    /// How large the window opens, when not `WIDTH` by `HEIGHT`: the
    /// application asks for a size of its own, and the window manager may
    /// give it another after its first frames.
    pub size: Option<(f32, f32)>,
}

impl Start {
    /// Started with a repository, as `bitbraid <path>` is.
    pub fn with(path: impl Into<PathBuf>) -> Self {
        Self {
            open: Some(path.into()),
            ..Self::default()
        }
    }

    /// Started with nothing.
    pub fn bare() -> Self {
        Self::default()
    }

    /// With this in `preferences.toml`.
    pub fn preferences(mut self, text: &str) -> Self {
        self.preferences = text.to_owned();
        self
    }

    /// With this in `state.toml`.
    pub fn state(mut self, text: impl Into<String>) -> Self {
        self.state = text.into();
        self
    }

    /// With this file in the configuration directory (`workspaces.toml`).
    pub fn config_file(mut self, name: &str, text: impl Into<String>) -> Self {
        self.files.push((name.to_owned(), text.into()));
        self
    }

    /// With frames drawn into images.
    pub fn pictures(mut self) -> Self {
        self.pictures = true;
        self
    }

    /// With a window that opens this large.
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = Some((width, height));
        self
    }
}

/// One window of the application.
pub struct Stage {
    cx: Option<HeadlessAppContext>,
    window: AnyWindowHandle,
    shell: Option<Entity<Shell>>,
    pictures: bool,
}

impl Stage {
    /// Start the application as `Start` says, in the scratch surroundings
    /// of this process, and wait until it is at rest.
    pub fn open(start: Start) -> Self {
        let (open, link) = (start.open.clone(), start.link.clone());
        let (mut stage, shell) =
            Self::open_root(start, move |window, cx| Shell::new(open, link, window, cx));
        stage.shell = Some(shell);
        stage
    }

    /// Start the application as `Start` says with a window of its own
    /// whose root is a view a test builds: a part of the window on its
    /// own, among the application's keys, theme and settings.
    pub fn open_root<V: Render>(
        start: Start,
        build: impl FnOnce(&mut Window, &mut gpui_kit::Context<V>) -> V + 'static,
    ) -> (Self, Entity<V>) {
        let root = crate::hermetic::inside().expect("the tests run in scratch surroundings");
        // The application's own directory, where `XDG_CONFIG_HOME` puts it.
        let config = root.join("config/bitbraid");
        if config.exists() {
            std::fs::remove_dir_all(&config).unwrap();
        }
        std::fs::create_dir_all(&config).unwrap();
        if !start.preferences.is_empty() {
            std::fs::write(config.join("preferences.toml"), &start.preferences).unwrap();
        }
        if !start.state.is_empty() {
            std::fs::write(config.join("state.toml"), &start.state).unwrap();
        }
        for (name, text) in &start.files {
            std::fs::write(config.join(name), text).unwrap();
        }

        let (width, height) = start.size.unwrap_or((WIDTH, HEIGHT));
        let text = text_system();
        text.add_fonts(vec![Cow::Borrowed(INTER), Cow::Borrowed(FIRA_MONO)])
            .expect("the fonts beside the tests are read");
        let assets = Arc::new(bitbraid_ui::app::Assets);
        let mut cx = if start.pictures {
            HeadlessAppContext::with_platform(text, assets, headless_renderer)
        } else {
            HeadlessAppContext::with_platform(text, assets, || Ok(None))
        };
        // The session threads answer from outside the test scheduler.
        cx.allow_parking();
        let (window, view) = cx
            .update(|cx| {
                bitbraid_ui::app::init_with(Prefs::load_from(Some(config)), cx);
                // What moves, moves on the real clock: nothing does here.
                cx.set_reduce_motion(true);
                // So does a scroll bar that shows for a moment after its
                // list was scrolled or while the pointer is over it, and
                // would be in a picture or not by how long the drawing
                // took: here it is always there where there is more to see.
                gpui_kit::component::Theme::set_scrollbar_mode(
                    gpui_kit::component::scroll::ScrollbarMode::Always,
                    cx,
                );
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: point(px(0.), px(0.)),
                            size: size(px(width), px(height)),
                        })),
                        focus: true,
                        show: true,
                        ..Default::default()
                    },
                    cx,
                    move |window, cx| cx.new(|cx| build(window, cx)),
                )
            })
            .expect("the window opens");
        let mut stage = Self {
            cx: Some(cx),
            window,
            shell: None,
            pictures: start.pictures,
        };
        stage.frame();
        (stage, view)
    }

    fn cx(&mut self) -> &mut HeadlessAppContext {
        self.cx.as_mut().expect("the application is running")
    }

    /// Do something with the window.
    pub fn window<R>(&mut self, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        let window = self.window;
        self.cx()
            .update_window(window, |_, window, cx| f(window, cx))
            .expect("the window is open")
    }

    /// Make the window this large and draw it, as the platform does when
    /// the window manager gives it another size.
    pub fn resize(&mut self, width: f32, height: f32) {
        self.window(|window, cx| {
            window.resize(size(px(width), px(height)));
            window.bounds_changed(cx);
            window.render_frame(cx);
        });
        self.frame();
    }

    /// Do something with the application.
    pub fn app<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R {
        self.cx().update(f)
    }

    /// Wait until the application is at rest: everything that was asked of
    /// a session thread is answered, and nothing is left to run on the main
    /// thread. A condition, not a length of time (`session::unanswered`).
    ///
    /// Two things reach the window as events and are not counted: what
    /// the file watcher saw, and the later pieces of a history so large
    /// that it is walked in pieces (the fixtures are not). A test that is
    /// about either waits for what it expects with [`Self::until`].
    pub fn settle(&mut self) {
        let started = Instant::now();
        loop {
            self.cx().run_until_parked();
            let owed = session::unanswered();
            if owed == 0 {
                // What the last replies woke has run now; if that asked
                // for nothing more, this is rest.
                self.cx().run_until_parked();
                if session::unanswered() == 0 {
                    return;
                }
                continue;
            }
            assert!(
                started.elapsed() < PATIENCE,
                "{owed} requests to the session threads are unanswered after {PATIENCE:?}"
            );
            session::wait_for_reply(owed, Duration::from_millis(250));
        }
    }

    /// Lay out and paint a frame, and settle whatever painting asked for
    /// (rows that come into view ask for their text).
    pub fn frame(&mut self) {
        // Twice: what the first frame measured, the second one uses.
        for _ in 0..2 {
            self.draw();
        }
    }

    fn draw(&mut self) {
        self.settle();
        self.window(|window, cx| {
            // The font of code is one of the two that are here; the
            // toolkit would look for one the machine has.
            if cx.global::<Theme>().mono_font_family != MONO_FONT {
                cx.global_mut::<Theme>().mono_font_family = MONO_FONT.into();
            }
            // So is that of the interface on macOS, where the system's
            // text system has the system's font for it.
            #[cfg(target_os = "macos")]
            if cx.global::<Theme>().font_family != UI_FONT {
                cx.global_mut::<Theme>().font_family = UI_FONT.into();
            }
            window.render_frame(cx);
        });
        self.settle();
    }

    /// Press keys as the toolkit writes them, one after the other:
    /// `"ctrl-p"`, `"down down enter"`.
    pub fn keys(&mut self, keys: &str) {
        for key in keys.split_whitespace() {
            self.window(|window, cx| window.press(key, cx));
            self.frame();
        }
    }

    /// Type text into whatever has the focus.
    pub fn type_text(&mut self, text: &str) {
        self.window(|window, cx| window.input(text, cx));
        self.frame();
    }

    /// Dispatch an action to whatever has the focus, as a menu entry or
    /// the palette does.
    pub fn act(&mut self, action: impl Action) {
        self.window(|window, cx| window.dispatch_action(Box::new(action), cx));
        self.frame();
    }

    /// Whether a control of this id is on show.
    pub fn shows(&mut self, id: impl Into<ElementId>) -> bool {
        self.window(|window, _| window.try_find(id).is_some_and(|found| found.visible()))
    }

    /// Click the control of this id with the pointer.
    pub fn click(&mut self, id: impl Into<ElementId>) {
        self.window(|window, cx| window.click(id, cx));
        self.frame();
    }

    /// Rest the pointer on the control of this id.
    pub fn hover(&mut self, id: impl Into<ElementId>) {
        self.window(|window, cx| window.hover(id, cx));
        self.frame();
    }

    /// Whether a toast is on show.
    pub fn shows_toast(&mut self) -> bool {
        self.shows("notification")
    }

    /// Put the toast away with its cross, which shows under the pointer.
    pub fn close_toast(&mut self) {
        self.hover("notification");
        self.window(|window, cx| window.within("notification").click("close", cx));
        self.frame();
    }

    /// Move the pointer off the window: nothing is under it any more, so
    /// what the next pictures show does not hang on where a click left it.
    pub fn pointer_away(&mut self) {
        self.window(|window, cx| {
            window.dispatch_event(
                MouseMoveEvent {
                    position: point(px(-100.), px(-100.)),
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
        });
        self.frame();
    }

    /// Click the control of this id at a place inside it, in points from
    /// its top left corner.
    pub fn click_in(&mut self, id: impl Into<ElementId>, (x, y): (f32, f32)) {
        self.window(|window, cx| window.click_at(id, point(px(x), px(y)), cx));
        self.frame();
    }

    /// Click the control of this id with the pointer's other button: what
    /// opens its menu.
    pub fn right_click(&mut self, id: impl Into<ElementId>) {
        self.window(|window, cx| window.right_click(id, cx));
        self.frame();
    }

    /// Where the control of this id is in the window, in points: its left
    /// and top edge, its width and its height.
    pub fn place(&mut self, id: impl Into<ElementId>) -> (f32, f32, f32, f32) {
        self.window(|window, _| {
            let bounds = window.find(id).bounds();
            (
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
            )
        })
    }

    /// Click the control `id` that is inside the control `scope`: the
    /// toolkit numbers the choices of a group from nought, whatever they
    /// were called.
    pub fn click_within(&mut self, scope: impl Into<ElementId>, id: impl Into<ElementId>) {
        self.window(|window, cx| window.within(scope).click(id, cx));
        self.frame();
    }

    /// Press and let go the pointer's button at a place in the window, in
    /// points: a click on whatever is there. For what has no id to be
    /// found by (a row of a virtual list).
    pub fn click_at(&mut self, place: (f32, f32)) {
        self.click_at_with(place, Default::default());
    }

    /// A click at a place with keys held: the secondary key adds to a
    /// selection, Shift makes a range.
    pub fn click_at_with(&mut self, (x, y): (f32, f32), modifiers: gpui_kit::Modifiers) {
        let position = point(px(x), px(y));
        self.window(|window, cx| {
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        });
        self.frame();
    }

    /// The last of `count` clicks in a row at a place, with keys held: two
    /// for a double click, three for a triple one.
    pub fn clicks_at(&mut self, (x, y): (f32, f32), count: usize, modifiers: gpui_kit::Modifiers) {
        let position = point(px(x), px(y));
        self.window(|window, cx| {
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: count,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: count,
                }
                .to_platform_input(),
                cx,
            );
        });
        self.frame();
    }

    /// Press and let go the other button of the pointer at a place in
    /// the window, in points: what opens the menu of whatever is there.
    /// For what has no id to be found by (a row of a virtual list, a tab).
    pub fn right_click_at(&mut self, (x, y): (f32, f32)) {
        let position = point(px(x), px(y));
        self.window(|window, cx| {
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Right,
                    position,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Right,
                    position,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        });
        self.frame();
    }

    /// Press the pointer's button at one place in the window, move it to
    /// another and let go there: a drag and a drop.
    pub fn drag_at(&mut self, from: (f32, f32), to: (f32, f32)) {
        self.window(|window, cx| {
            window.drag(point(px(from.0), px(from.1)), point(px(to.0), px(to.1)), cx)
        });
        self.frame();
    }

    /// Do something with the window's root, in the window.
    pub fn with_shell<R>(
        &mut self,
        f: impl FnOnce(&mut Shell, &mut Window, &mut gpui_kit::Context<Shell>) -> R,
    ) -> R {
        let shell = self.shell.clone().expect("the shell is there");
        self.window(|window, cx| shell.update(cx, |shell, cx| f(shell, window, cx)))
    }

    /// Where a row of the graph of the shown tab is in the window: on its
    /// first label, or on its message.
    pub fn graph_spot(&mut self, row: usize, on_label: bool) -> (f32, f32) {
        let shell = self.shell.clone().expect("the shell is there");
        self.app(|cx| {
            let workspace = shell
                .read(cx)
                .shown_workspace()
                .expect("the shown tab is a repository's");
            let graph = workspace.read(cx).graph();
            graph.read(cx).spot(row, on_label)
        })
    }

    /// Where the heading of a column of the graph of the shown tab is.
    pub fn graph_heading(&mut self, column: bitbraid_ui::prefs::Column) -> (f32, f32) {
        let shell = self.shell.clone().expect("the shell is there");
        self.app(|cx| {
            let workspace = shell
                .read(cx)
                .shown_workspace()
                .expect("the shown tab is a repository's");
            let graph = workspace.read(cx).graph();
            graph
                .read(cx)
                .heading_spot(column, cx)
                .expect("the column is shown")
        })
    }

    /// Where the field of the row of uncommitted work of the shown tab is.
    pub fn graph_wip_field(&mut self) -> (f32, f32) {
        let shell = self.shell.clone().expect("the shell is there");
        self.app(|cx| {
            let workspace = shell
                .read(cx)
                .shown_workspace()
                .expect("the shown tab is a repository's");
            let graph = workspace.read(cx).graph();
            graph.read(cx).wip_field_spot(cx)
        })
    }

    /// What the labels of a row of the graph of the shown tab say.
    pub fn graph_labels(&mut self, row: usize) -> Vec<String> {
        let shell = self.shell.clone().expect("the shell is there");
        self.app(|cx| {
            let workspace = shell
                .read(cx)
                .shown_workspace()
                .expect("the shown tab is a repository's");
            let graph = workspace.read(cx).graph();
            graph.read(cx).label_names(row)
        })
    }

    /// Press the pointer's button at one place and move it, held, to
    /// another, without letting go: a drag under way. `release_at` ends it.
    pub fn press_and_move(&mut self, from: (f32, f32), to: (f32, f32)) {
        let (start, end) = (point(px(from.0), px(from.1)), point(px(to.0), px(to.1)));
        self.window(|window, cx| {
            window.dispatch_event(
                MouseMoveEvent {
                    position: start,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position: start,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            // In two steps: the first starts the drag, the second is a
            // move while it is under way.
            for position in [point((start.x + end.x) / 2.0, (start.y + end.y) / 2.0), end] {
                window.dispatch_event(
                    MouseMoveEvent {
                        position,
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                window.render_frame(cx);
            }
        });
        self.frame();
    }

    /// Let go of the pointer's button at a place: the end of a drag.
    pub fn release_at(&mut self, (x, y): (f32, f32)) {
        let position = point(px(x), px(y));
        self.window(|window, cx| {
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        });
        self.frame();
    }

    /// Move the pointer to a place in the window, in points, without a
    /// button: what is under it hears that the pointer is on it.
    pub fn point_at(&mut self, (x, y): (f32, f32)) {
        let position = point(px(x), px(y));
        self.window(|window, cx| {
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
        });
        self.frame();
    }

    /// Let the timers of the application run that are due within this
    /// long. Their clock is the test scheduler's and stands still until it
    /// is moved.
    pub fn pass(&mut self, time: Duration) {
        self.cx().advance_clock(time);
        self.frame();
    }

    /// Draw frames until `met` holds, for what follows a change on disk:
    /// the watcher sees it a moment later, on a thread of its own.
    pub fn until(&mut self, what: &str, mut met: impl FnMut(&mut Self) -> bool) {
        let started = Instant::now();
        loop {
            self.frame();
            if met(self) {
                return;
            }
            assert!(
                started.elapsed() < PATIENCE,
                "waited {PATIENCE:?} for this and it did not come: {what}"
            );
            std::thread::park_timeout(Duration::from_millis(20));
        }
    }

    /// What the window has on show.
    pub fn shell(&mut self) -> ShellProbe {
        let shell = self.shell.clone().expect("the shell is there");
        self.app(|cx| shell.read(cx).probe(cx))
    }

    /// What the terminal of the shown tab has on show.
    pub fn terminal(&mut self) -> bitbraid_ui::views::terminal::probe::TerminalProbe {
        let shell = self.shell.clone().expect("the shell is there");
        self.window(|window, cx| shell.read(cx).terminal_probe(window, cx))
    }

    /// What the shown tab has on show. Panics when it is no repository's.
    pub fn workspace(&mut self) -> WorkspaceProbe {
        let shell = self.shell.clone().expect("the shell is there");
        self.app(|cx| {
            let workspace = shell
                .read(cx)
                .shown_workspace()
                .expect("the shown tab is a repository's");
            workspace.read(cx).probe(cx)
        })
    }

    /// What the graph of the shown tab has on show.
    pub fn graph(&mut self) -> GraphProbe {
        let shell = self.shell.clone().expect("the shell is there");
        self.app(|cx| {
            let workspace = shell
                .read(cx)
                .shown_workspace()
                .expect("the shown tab is a repository's");
            let graph = workspace.read(cx).graph();
            graph.read(cx).probe(cx)
        })
    }

    /// Do something with the workspace of the shown tab, in the window.
    /// Panics when the tab is no repository's.
    pub fn with_workspace<R>(
        &mut self,
        f: impl FnOnce(&Entity<Workspace>, &mut Window, &mut App) -> R,
    ) -> R {
        let shell = self.shell.clone().expect("the shell is there");
        self.window(|window, cx| {
            let workspace = shell
                .read(cx)
                .shown_workspace()
                .expect("the shown tab is a repository's");
            f(&workspace, window, cx)
        })
    }

    /// Put the keyboard in the changes panel of the shown tab, as a click
    /// on it does. Its rows are then walked with its own keys.
    pub fn focus_changes(&mut self) {
        self.with_workspace(|workspace, window, cx| {
            let changes = workspace.read(cx).changes();
            changes.read(cx).focus_handle(cx).focus(window, cx);
        });
        self.frame();
    }

    /// Open the changed file `path` in the centre with the keys of the
    /// changes panel: from the first row down, Enter on each, until the
    /// file in the centre is that one.
    pub fn open_changed(&mut self, path: &str) {
        self.focus_changes();
        self.keys("home");
        for _ in 0..64 {
            self.focus_changes();
            self.keys("enter");
            let shown = self.workspace().file.map(|(shown, _)| shown);
            if shown.as_deref() == Some(path) {
                return;
            }
            self.focus_changes();
            self.keys("down");
        }
        panic!("the changes panel has no row that opens {path}");
    }

    /// Where the keys go in the graph of the shown tab: `(rows, search)`.
    pub fn graph_focus(&mut self) -> (bool, bool) {
        let shell = self.shell.clone().expect("the shell is there");
        self.window(|window, cx| {
            let workspace = shell
                .read(cx)
                .shown_workspace()
                .expect("the shown tab is a repository's");
            let graph = workspace.read(cx).graph();
            let graph = graph.read(cx);
            (
                graph.rows_focused(window, cx),
                graph.search_focused(window, cx),
            )
        })
    }

    /// Everything that was said in notifications, oldest first.
    pub fn notices(&mut self) -> Vec<String> {
        self.app(|cx| {
            cx.global::<bitbraid_ui::notices::Notices>()
                .entries
                .iter()
                .map(|entry| entry.text.to_string())
                .collect()
        })
    }

    /// Whether a dialog of the toolkit is open over the window.
    pub fn dialog_open(&mut self) -> bool {
        use gpui_kit::component::WindowExt as _;
        self.window(|window, cx| window.has_active_dialog(cx))
    }

    /// Show the window in one of the built-in themes.
    pub fn use_theme(&mut self, theme: &ThemeChoice) {
        // Drawn with the next frame.
        self.app(|cx| bitbraid_ui::theme::apply(theme, cx));
    }

    /// Draw the window as it is now. The frame is drawn until two in a row
    /// are the same picture, so that nothing half-arrived is in it.
    pub fn picture(&mut self) -> RgbaImage {
        assert!(self.pictures, "this window was opened without pictures");
        let started = Instant::now();
        let mut last: Option<RgbaImage> = None;
        loop {
            self.draw();
            let window = self.window;
            let frame = self
                .cx()
                .capture_screenshot(window)
                .expect("the frame is drawn into an image");
            if last
                .as_ref()
                .is_some_and(|last| last.as_raw() == frame.as_raw())
            {
                return frame;
            }
            assert!(
                started.elapsed() < PATIENCE,
                "the picture was still changing after {PATIENCE:?}"
            );
            last = Some(frame);
        }
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        self.shell = None;
        if let Some(cx) = self.cx.take() {
            if std::thread::panicking() {
                // A test that failed half-way leaves the application in a
                // state nobody vouches for; it is not taken down in order.
                std::mem::forget(cx);
            } else {
                drop(cx);
            }
        }
    }
}
