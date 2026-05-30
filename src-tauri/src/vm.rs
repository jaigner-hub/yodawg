//! VM configuration model and on-disk persistence.
//!
//! Each VM lives in its own directory under the app data dir:
//!
//! ```text
//! <app_data>/machines/<name>/
//!   ├── vm.json     # this VmConfig, serialized
//!   └── disk.qcow2  # the virtual disk
//! ```

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Persistent configuration for a single virtual machine.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VmConfig {
    pub name: String,
    pub memory_mb: u32,
    pub cpus: u32,
    /// Absolute path to the qcow2 disk image.
    pub disk_path: String,
    pub disk_size_gb: u32,
    /// Absolute path to an install ISO, if one is attached.
    #[serde(default)]
    pub iso_path: Option<String>,
}

/// The directory holding all machine subdirectories.
pub fn machines_dir(app_data: &Path) -> PathBuf {
    app_data.join("machines")
}

/// The directory for a single named machine.
pub fn machine_dir(app_data: &Path, name: &str) -> PathBuf {
    machines_dir(app_data).join(name)
}

/// Persist a VM config to its `vm.json`, creating the machine directory.
pub fn save(app_data: &Path, cfg: &VmConfig) -> Result<(), String> {
    let dir = machine_dir(app_data, &cfg.name);
    fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(dir.join("vm.json"), json).map_err(|e| e.to_string())
}

/// Load a single VM config by name.
pub fn load(app_data: &Path, name: &str) -> Result<VmConfig, String> {
    let path = machine_dir(app_data, name).join("vm.json");
    let json = fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    serde_json::from_str(&json).map_err(|e| format!("parsing {}: {e}", path.display()))
}

/// Load every VM config found under the machines directory.
pub fn load_all(app_data: &Path) -> Vec<VmConfig> {
    let dir = machines_dir(app_data);
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(), // no machines dir yet -> empty list
    };
    let mut vms = Vec::new();
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                if let Ok(cfg) = load(app_data, name) {
                    vms.push(cfg);
                }
            }
        }
    }
    vms.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    vms
}

/// Delete a machine directory and everything in it.
pub fn delete(app_data: &Path, name: &str) -> Result<(), String> {
    let dir = machine_dir(app_data, name);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}
