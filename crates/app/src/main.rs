use gpui_kit::*;
use rsdisk_app::view::RsDisk;
use std::path::PathBuf;

fn main() {
    let home = std::env::home_dir().expect("no home folder");
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(home);
    let root = std::path::absolute(&root).unwrap_or(root);
    // Screenshot launches must not take keyboard focus from whatever Tim is typing in. They
    // float above other windows instead: macOS stops redrawing a covered window.
    let focus = std::env::var_os("RSDISK_NO_FOCUS").is_none();
    let kind = if focus {
        WindowKind::Normal
    } else {
        WindowKind::PopUp
    };

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
                kind,
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| RsDisk::new(root.clone(), window, cx)),
        )
        .expect("couldn't open the window");
        if focus {
            cx.activate(true);
        }
    });
}
