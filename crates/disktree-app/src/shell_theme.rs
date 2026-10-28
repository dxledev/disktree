//! The palette of the active desktop shell.
//!
//! The desktop keeps one palette directory per theme: the static themes in
//! `~/.config/themes/<name>`, and one generated per shell (caelestia, island,
//! noctalia) in `~/.config/themes/.dynamic/<shell>`. A selector symlink,
//! `~/.config/themes/.caelestia-use`, points at whichever is active and is
//! repointed on every theme or shell-mode change, waybar mode included. The
//! other apps on the desktop follow that one link rather than knowing the
//! shells, and so does this one: every palette directory carries a
//! `Colors.qml` with the same roles, and dynamic ones add a `palette.json`
//! that states light or dark.
//!
//! gpui-omarchy only reads Omarchy's own `colors.toml`; here the palette is
//! translated into that shape and handed to it, so the rest of the app sees
//! an ordinary [`Theme`].

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::App;
use gpui_omarchy::Theme;

/// The selector the desktop repoints, under the themes directory.
const SELECTOR: &str = ".caelestia-use";

/// How often the selector and its palette are re-read. Switching rewrites
/// symlinks and files in several steps, so watching single paths misses
/// replacements; a second is imperceptible and the read is a few kilobytes.
const POLL: Duration = Duration::from_secs(1);

/// The themes directory, honouring `XDG_CONFIG_HOME` like the desktop does.
pub fn themes_dir(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
        .join("themes")
}

/// What one look at the themes directory found.
#[derive(Debug, PartialEq, Eq)]
pub enum Reading {
    /// No desktop theme setup here at all.
    Absent,
    /// A setup that cannot be read right now: a dangling selector or a
    /// half-written palette in the middle of a switch. The caller keeps what
    /// it has rather than flashing a default.
    Pending,
    /// The active palette, as the `colors.toml` gpui-omarchy understands.
    Palette { name: String, colors: String },
}

/// Read the active palette: the selector if it exists, else the static
/// `current` theme.
pub fn read(themes: &Path) -> Reading {
    let selector = themes.join(SELECTOR);
    let dir = if fs::symlink_metadata(&selector).is_ok() {
        selector
    } else {
        let current = themes.join("current");
        if fs::symlink_metadata(&current).is_err() {
            return Reading::Absent;
        }
        current
    };
    let Ok(real) = fs::canonicalize(&dir) else {
        return Reading::Pending;
    };
    colors_toml(&real).map_or(Reading::Pending, |colors| Reading::Palette {
        name: describe(&real),
        colors,
    })
}

/// "akane", or "dynamic island" for a shell's generated palette.
fn describe(dir: &Path) -> String {
    let name = dir
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let dynamic = dir
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|parent| parent == ".dynamic");
    if dynamic {
        format!("dynamic {name}")
    } else {
        name
    }
}

/// Translate a palette directory into Omarchy's `colors.toml` roles.
fn colors_toml(dir: &Path) -> Option<String> {
    let qml = qml_colors(&fs::read_to_string(dir.join("Colors.qml")).ok()?);
    let json = fs::read_to_string(dir.join("palette.json")).ok();
    let json_color = |key: &str| json.as_deref().and_then(|j| json_hex(j, key));
    let pick = |names: &[&str], fallback: &str| {
        names
            .iter()
            .find_map(|name| qml.get(*name).cloned())
            .or_else(|| json_color(fallback))
    };
    // `backgroundAlt` before `background`: a static theme's `background` is
    // the translucent panel colour, with the alpha first in QML's #AARRGGBB.
    let mut out = String::new();
    for (role, value) in [
        (
            "background",
            pick(&["backgroundAlt", "background"], "base")?,
        ),
        ("foreground", pick(&["foreground"], "text")?),
        ("accent", pick(&["primary"], "primary")?),
        ("red", pick(&["danger"], "red")?),
        ("yellow", pick(&["warning"], "yellow")?),
        ("green", pick(&["success"], "green")?),
    ] {
        let _ = writeln!(out, "{role} = \"{value}\"");
    }
    if let Some(value) = qml.get("backgroundGray") {
        let _ = writeln!(out, "lighter_background = \"{value}\"");
    }
    // Only dynamic palettes say light or dark; for a static one the
    // luminance of its background and foreground decides.
    if let Some(mode @ ("light" | "dark")) =
        json.as_deref().and_then(|j| json_string(j, "mode"))
    {
        let _ = writeln!(out, "mode = \"{mode}\"");
    }
    Some(out)
}

/// `#rrggbb` from `#rrggbb`, `rrggbb` or QML's `#aarrggbb`.
fn hex(value: &str) -> Option<String> {
    let digits = value.trim().trim_start_matches('#');
    if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = match digits.len() {
        6 => digits,
        8 => &digits[2..],
        _ => return None,
    };
    Some(format!("#{}", rgb.to_ascii_lowercase()))
}

/// The `readonly property color name: "#hex"` lines of a `Colors.qml`.
/// Computed colours such as `Qt.rgba(...)` are skipped.
fn qml_colors(contents: &str) -> HashMap<String, String> {
    contents
        .lines()
        .filter_map(|line| {
            let (head, value) = line.split_once(':')?;
            if !head.contains("property color") {
                return None;
            }
            let name = head.split_whitespace().last()?;
            Some((name.to_owned(), hex(value.trim().trim_matches('"'))?))
        })
        .collect()
}

/// A top-level string value from a flat JSON object, without a JSON parser:
/// the palettes are generated one `"key": "value"` pair to a line.
fn json_string<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("\"{key}\"");
    contents.lines().find_map(|line| {
        let value = line.trim().strip_prefix(&prefix)?;
        let value = value.trim_start().strip_prefix(':')?;
        value
            .trim()
            .trim_end_matches(',')
            .trim()
            .strip_prefix('"')?
            .strip_suffix('"')
    })
}

fn json_hex(contents: &str, key: &str) -> Option<String> {
    hex(json_string(contents, key)?)
}

/// Apply `reading` unless it is what is already applied. Only a palette that
/// gpui-omarchy accepts replaces `applied`, so a bad one leaves the last good
/// theme on screen.
fn apply(
    reading: Reading,
    applied: &mut Option<(String, String)>,
    cx: &mut App,
) {
    let Reading::Palette { name, colors } = reading else {
        return;
    };
    let key = (name, colors);
    if applied.as_ref() == Some(&key) {
        return;
    }
    if let Ok(theme) = Theme::from_colors_toml(&key.0, &key.1) {
        theme.apply(cx);
        *applied = Some(key);
    }
}

/// Apply the active shell palette and follow it from now on. Returns whether
/// this machine has the desktop's theme setup at all; if not, nothing was
/// done and the caller falls back to Omarchy or the system appearance.
///
/// Applying an explicit theme stops gpui-omarchy's own watcher, which would
/// otherwise fight this one for the same globals.
pub fn follow(home: &Path, cx: &mut App) -> bool {
    let themes = themes_dir(home);
    let first = read(&themes);
    if first == Reading::Absent {
        return false;
    }
    let mut applied = None;
    apply(first, &mut applied, cx);
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(POLL).await;
            let dir = themes.clone();
            let reading = cx
                .background_executor()
                .spawn(async move { read(&dir) })
                .await;
            cx.update(|cx| apply(reading, &mut applied, cx));
        }
    })
    .detach();
    true
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    const STATIC_QML: &str = r##"pragma Singleton
Singleton {
    readonly property color background: "#e00e1e36"
    readonly property color backgroundAlt: "#0E1E36"
    readonly property color backgroundAlpha: Qt.rgba(26 / 255, 42 / 255, 70 / 255, 0.88)
    readonly property color backgroundGray: "#2a3a56"
    readonly property color foreground: "#f4b999"
    readonly property color primary: "#9279aa"
    readonly property color success: "#be6f76"
    readonly property color warning: "#f4b999"
    readonly property color danger: "#d2495b"
}
"##;

    fn palette_dir(root: &Path, name: &str, qml: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Colors.qml"), qml).unwrap();
        dir
    }

    #[test]
    fn qml_alpha_is_dropped_and_computed_colours_skipped() {
        let colors = qml_colors(STATIC_QML);
        assert_eq!(colors["background"], "#0e1e36");
        assert!(!colors.contains_key("backgroundAlpha"));
        assert_eq!(hex("e7beaf").as_deref(), Some("#e7beaf"));
        assert_eq!(hex("#12345"), None);
        assert_eq!(hex("#ggggggg"), None);
    }

    #[test]
    fn a_static_theme_loads_without_a_mode() {
        let root = tempfile::tempdir().unwrap();
        palette_dir(root.path(), "akane", STATIC_QML);
        symlink(root.path().join("akane"), root.path().join("current"))
            .unwrap();
        let Reading::Palette { name, colors } = read(root.path()) else {
            panic!("static theme not read");
        };
        assert_eq!(name, "akane");
        assert!(colors.contains("background = \"#0e1e36\""));
        assert!(!colors.contains("mode"));
        let theme = Theme::from_colors_toml(&name, &colors).unwrap();
        assert!(matches!(
            theme.appearance,
            gpui_kit::base::ThemeAppearance::Dark
        ));
    }

    #[test]
    fn the_selector_beats_current_and_a_dynamic_mode_is_used() {
        let root = tempfile::tempdir().unwrap();
        palette_dir(root.path(), "akane", STATIC_QML);
        symlink(root.path().join("akane"), root.path().join("current"))
            .unwrap();
        let dynamic = palette_dir(
            &root.path().join(".dynamic"),
            "island",
            &STATIC_QML.replace("#0E1E36", "#fbf1ea"),
        );
        fs::write(dynamic.join("palette.json"), "{\n  \"mode\": \"light\",\n")
            .unwrap();
        symlink(".dynamic/island", root.path().join(SELECTOR)).unwrap();
        let Reading::Palette { name, colors } = read(root.path()) else {
            panic!("dynamic theme not read");
        };
        assert_eq!(name, "dynamic island");
        assert!(colors.contains("mode = \"light\""));
    }

    #[test]
    fn a_dangling_selector_is_pending_not_absent() {
        let root = tempfile::tempdir().unwrap();
        symlink(".dynamic/gone", root.path().join(SELECTOR)).unwrap();
        assert_eq!(read(root.path()), Reading::Pending);
    }

    #[test]
    fn a_half_written_palette_is_pending() {
        let root = tempfile::tempdir().unwrap();
        palette_dir(root.path(), "new", "Singleton {\n");
        symlink("new", root.path().join(SELECTOR)).unwrap();
        assert_eq!(read(root.path()), Reading::Pending);
    }

    #[test]
    fn no_themes_directory_is_absent() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(read(root.path()), Reading::Absent);
    }
}
