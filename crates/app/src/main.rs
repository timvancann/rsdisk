use gpui_kit::*;
use rsdisk_app::settings_window;
use rsdisk_app::view::RsDisk;
use std::path::PathBuf;

actions!(rsdisk, [OpenSettings, Quit]);

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

    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            let bounds = Bounds::centered(None, size(px(1200.), px(780.)), cx);
            let title = format!("rsdisk: {}", root.display());
            let (_, main) = gpui_kit::open_window(
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

            let weak = main.downgrade();
            cx.on_action(move |_: &OpenSettings, cx| settings_window::open(weak.clone(), cx));
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("cmd-,", OpenSettings, None),
                KeyBinding::new("cmd-q", Quit, None),
            ]);
            cx.set_menus(vec![Menu::new("RSdisk").items([
                MenuItem::action("Settings\u{2026}", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Quit RSdisk", Quit),
            ])]);
            // Screenshots of the settings window (spec §3.15): open it at launch.
            if std::env::var_os("RSDISK_SHOW_SETTINGS").is_some() {
                settings_window::open(main.downgrade(), cx);
            }
            if focus {
                cx.activate(true);
            }
        });
}
