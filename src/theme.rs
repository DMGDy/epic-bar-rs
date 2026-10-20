use std::{
    cell::RefCell,
    env, fs,
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::CssProvider;

fn first_existing(paths: &[PathBuf]) -> Result<PathBuf, String> {
    for path in paths {
        match path.try_exists() {
            Ok(true) => return Ok(path.clone()),
            Ok(false) => {}
            Err(error) => return Err(format!("cannot access theme {}: {error}", path.display())),
        }
    }
    Err(format!(
        "no theme found; set EPIC_BAR_THEME to a CSS file (searched: {})",
        paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn theme_path() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("EPIC_BAR_THEME") {
        if path.is_empty() {
            return Err("EPIC_BAR_THEME must name a CSS file".into());
        }
        return Ok(PathBuf::from(path));
    }

    let mut paths = Vec::new();
    let config_dir = env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    if let Some(config_dir) = config_dir {
        paths.push(config_dir.join("epic-bar-rs/theme.css"));
    }
    let executable =
        env::current_exe().map_err(|error| format!("cannot locate executable: {error}"))?;
    if let Some(prefix) = executable.parent().and_then(Path::parent) {
        paths.push(prefix.join("share/epic-bar-rs/themes/autumn-light.css"));
    }
    paths.push(PathBuf::from("theme.css"));
    first_existing(&paths)
}

fn load_file(path: &Path) -> Result<CssProvider, String> {
    fs::read(path).map_err(|error| format!("cannot read theme {}: {error}", path.display()))?;
    let provider = CssProvider::new();
    let errors = Rc::new(RefCell::new(Vec::new()));
    let parser_errors = errors.clone();
    provider.connect_parsing_error(move |_, section, error| {
        parser_errors
            .borrow_mut()
            .push(format!("{section}: {error}"));
    });
    provider.load_from_path(path);
    if errors.borrow().is_empty() {
        Ok(provider)
    } else {
        Err(format!(
            "invalid theme {}:\n{}",
            path.display(),
            errors.borrow().join("\n")
        ))
    }
}

pub fn load() -> Result<CssProvider, String> {
    load_file(&theme_path()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_first_existing_theme() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let saved = root.join("themes/windows-11-light.css");
        let autumn = root.join("themes/autumn-light.css");
        assert_eq!(
            first_existing(&[saved.clone(), autumn.clone()]).unwrap(),
            saved
        );
        assert_eq!(
            first_existing(&[root.join("themes/missing.css"), autumn.clone()]).unwrap(),
            autumn
        );
        assert!(first_existing(&[root.join("themes/missing.css")]).is_err());
    }

    #[test]
    #[ignore = "requires a GTK display"]
    fn parses_themes_and_reports_errors() {
        gtk::init().expect("GTK display required");
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for theme in [
            "theme.css",
            "themes/windows-11-light.css",
            "themes/autumn-light.css",
        ] {
            load_file(&root.join(theme)).unwrap();
        }
        assert!(load_file(&root.join("themes/missing.css")).is_err());
        let invalid =
            env::temp_dir().join(format!("epic-bar-invalid-theme-{}.css", std::process::id()));
        fs::write(
            &invalid,
            "@import url(\"missing-theme.css\");\nbutton { color: not-a-color; }",
        )
        .unwrap();
        let error = load_file(&invalid).err().expect("invalid CSS must fail");
        fs::remove_file(invalid).unwrap();
        assert!(error.contains("invalid theme"));
    }
}
