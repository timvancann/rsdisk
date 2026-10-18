use gpui_kit::*;
use std::path::PathBuf;

struct Shell {
    root: PathBuf,
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(self.root.display().to_string())
    }
}

fn main() {
    let home = std::env::home_dir().expect("no home folder");
    let root = std::env::args_os().nth(1).map(PathBuf::from).unwrap_or(home);
    let root = std::path::absolute(&root).unwrap_or(root);
    // Screenshot launches must not take keyboard focus from whatever Tim is typing in.
    let focus = std::env::var_os("RSDISK_NO_FOCUS").is_none();

    application().run(move |cx: &mut App| {
        gpui_kit::init(cx);
        let bounds = Bounds::centered(None, size(px(1200.), px(780.)), cx);
        let title = format!("rsdisk: {}", root.display());
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(title.into()),
                    ..Default::default()
                }),
                focus,
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|_| Shell { root: root.clone() }),
        )
        .expect("couldn't open the window");
        if focus {
            cx.activate(true);
        }
    });
}
