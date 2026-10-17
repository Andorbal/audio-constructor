//! Loading and saving `.acproj` files.

use std::path::Path;

use ac_core::Project;

/// Read a project file. Errors are human-readable and never panic.
pub fn load_project(path: &Path) -> Result<Project, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Project::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Write a project file in canonical form, atomically (write a temp file next
/// to it, then rename) so a crash never leaves a half-written project.
pub fn save_project(path: &Path, project: &Project) -> Result<(), String> {
    let text = project.to_json();
    let tmp = path.with_extension("acproj.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("ac-proj-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.acproj");
        let proj = Project {
            name: "X".into(),
            ..Project::default()
        };
        save_project(&p, &proj).unwrap();
        assert_eq!(load_project(&p).unwrap(), proj);
        assert!(load_project(&dir.join("missing.acproj")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
