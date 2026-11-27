//! Ambiente del processo, sistemato prima che parta la GUI (solo Linux: altrove non c'è glibc né il
//! loader Vulkan da regolare).
//!
//! glibc e il loader Vulkan leggono le loro variabili solo all'avvio, e `std::env::set_var` è
//! `unsafe` (il workspace lo vieta): per cambiarle il client si rilancia una volta con `exec`, che
//! mantiene il PID. Misurato su Linux con lyrics aperte (RSS dopo 15 s di riproduzione):
//!
//! | Ambiente | RSS |
//! |---|---|
//! | predefinito | 367–388 MB |
//! | `MALLOC_ARENA_MAX=2` | 295 MB |
//! | + niente driver Vulkan NVIDIA né OpenGL | 222 MB |

/// Chi la trova impostata è già passato di qui: si evita di rilanciarsi all'infinito.
const MARKER: &str = "PSYTIFY_ENV_READY";

/// Se una di queste è impostata, l'utente ha scelto lui GPU e backend: non si tocca nulla.
const GPU_OPT_OUT: [&str; 10] = [
    "PSYTIFY_ALL_GPUS",
    "VK_LOADER_DRIVERS_SELECT",
    "VK_LOADER_DRIVERS_DISABLE",
    "VK_ICD_FILENAMES",
    "VK_DRIVER_FILES",
    "WGPU_BACKEND",
    "WGPU_ADAPTER_NAME",
    "WGPU_POWER_PREF",
    "__NV_PRIME_RENDER_OFFLOAD",
    "DRI_PRIME",
];

/// Driver Vulkan delle GPU integrate (RADV, ANV, AMDVLK): se ce n'è uno, iced ci disegna già
/// (chiede la GPU a basso consumo) e il driver NVIDIA si carica per niente.
const INTEGRATED_DRIVERS: [&str; 3] = ["radeon", "intel", "amd"];

/// I file che descrivono i driver Vulkan installati.
const VULKAN_ICD_DIRS: [&str; 3] =
    ["/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d", "/usr/local/share/vulkan/icd.d"];

/// Le variabili da impostare, dati i driver Vulkan installati e quelle già presenti.
fn lean_env(manifests: &[String], is_set: impl Fn(&str) -> bool) -> Vec<(&'static str, &'static str)> {
    let mut changes = Vec::new();
    // Un thread = una arena malloc: con 40 thread glibc ne apre 32 e tiene ~75 MB liberi ma non resi.
    if !is_set("MALLOC_ARENA_MAX") {
        changes.push(("MALLOC_ARENA_MAX", "2"));
    }
    if leaves_nvidia_driver_unloaded(manifests, &is_set) {
        // Solo Vulkan: l'OpenGL di wgpu caricherebbe comunque le librerie EGL di NVIDIA e di Mesa.
        changes.push(("VK_LOADER_DRIVERS_DISABLE", "*nvidia*"));
        changes.push(("WGPU_BACKEND", "vulkan"));
    }
    changes
}

/// C'è un driver NVIDIA e uno di una GPU integrata, e l'utente non ha scelto la GPU a mano.
fn leaves_nvidia_driver_unloaded(manifests: &[String], is_set: &impl Fn(&str) -> bool) -> bool {
    if GPU_OPT_OUT.iter().any(|name| is_set(name)) {
        return false;
    }
    let has = |vendor: &str| manifests.iter().any(|manifest| manifest.contains(vendor));
    has("nvidia") && INTEGRATED_DRIVERS.iter().any(|vendor| has(vendor))
}

/// I nomi (in minuscolo) dei file dei driver Vulkan installati.
fn installed_vulkan_drivers() -> Vec<String> {
    VULKAN_ICD_DIRS
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
        .collect()
}

/// Rilancia il client con l'ambiente più leggero, se serve. Va chiamata per prima in `main`.
pub fn relaunch_with_lean_env() {
    use std::os::unix::process::CommandExt;

    if std::env::var_os(MARKER).is_some() {
        return;
    }
    let changes = lean_env(&installed_vulkan_drivers(), |name| std::env::var_os(name).is_some());
    if changes.is_empty() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    // `exec` torna solo se fallisce: in quel caso il client parte comunque, con l'ambiente di prima.
    let _failed = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(MARKER, "1")
        .envs(changes)
        .exec();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drivers(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn nothing_set(_: &str) -> bool {
        false
    }

    #[test]
    fn the_malloc_arenas_are_capped_unless_the_user_chose() {
        assert!(lean_env(&[], nothing_set).contains(&("MALLOC_ARENA_MAX", "2")));
        assert!(lean_env(&[], |name| name == "MALLOC_ARENA_MAX").is_empty());
    }

    #[test]
    fn the_nvidia_driver_is_skipped_when_an_integrated_one_is_there() {
        let changes = lean_env(&drivers(&["nvidia_icd.json", "radeon_icd.json"]), nothing_set);
        assert!(changes.contains(&("VK_LOADER_DRIVERS_DISABLE", "*nvidia*")));
        assert!(changes.contains(&("WGPU_BACKEND", "vulkan")));
    }

    #[test]
    fn an_nvidia_only_machine_keeps_its_driver() {
        let changes = lean_env(&drivers(&["nvidia_icd.json", "lvp_icd.json"]), nothing_set);
        assert!(!changes.iter().any(|(name, _)| *name == "VK_LOADER_DRIVERS_DISABLE"));
    }

    #[test]
    fn a_machine_without_nvidia_is_left_alone() {
        let changes = lean_env(&drivers(&["intel_icd.json"]), nothing_set);
        assert!(!changes.iter().any(|(name, _)| *name == "WGPU_BACKEND"));
    }

    #[test]
    fn the_gpu_is_not_touched_when_the_user_picked_one() {
        let manifests = drivers(&["nvidia_icd.json", "radeon_icd.json"]);
        for opt_out in ["DRI_PRIME", "__NV_PRIME_RENDER_OFFLOAD", "WGPU_BACKEND", "PSYTIFY_ALL_GPUS"] {
            let changes = lean_env(&manifests, |name| name == opt_out);
            assert!(!changes.iter().any(|(name, _)| *name == "VK_LOADER_DRIVERS_DISABLE"), "{opt_out}");
        }
    }
}
