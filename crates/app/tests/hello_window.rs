//! The hello window, rendered through Rune's real runtime with no window.

use herdr_frontend::{SIZE, TITLE, spec};
use rune::{App, Env};

#[test]
fn the_window_is_dark_and_titled_herdr_frontend() {
    let spec = spec();
    assert_eq!(spec.title, TITLE);
    assert_eq!(TITLE, "herdr-frontend");

    let mut rt = App::new(spec).headless_runtime(SIZE, Env::new(true, 2.0));
    rt.frame(0.0);
    let elements = rt.elements();
    let by_key = |key: &str| {
        elements
            .iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("no element keyed {key}"))
    };

    let root = by_key("root");
    assert_eq!(
        root.rect,
        [0.0, 0.0, SIZE.0, SIZE.1],
        "the root fills the window"
    );
    assert_eq!(root.fill.a, 1.0, "the background is opaque");
    assert!(
        root.fill.luminance() < 0.05,
        "the background is dark: {:?}",
        root.fill
    );

    assert_eq!(by_key("title").text.as_deref(), Some(TITLE));
}
