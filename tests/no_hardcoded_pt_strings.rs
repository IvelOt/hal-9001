use std::fs;
use std::path::Path;

fn check_dir(dir: &Path, errors: &mut Vec<String>) {
    if dir.is_dir() {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                check_dir(&path, errors);
            } else if path.extension().unwrap_or_default() == "rs" {
                let content = fs::read_to_string(&path).unwrap();
                
                // Let's look for known remaining hardcoded PT strings as a proof
                if content.contains("coletando dados do sistema") {
                    errors.push(format!("{}: contains 'coletando dados do sistema'", path.display()));
                }
                if content.contains("Monitor {name} conectado. Ativando modo Expandir") {
                    errors.push(format!("{}: contains 'Monitor {{name}} conectado. Ativando modo Expandir'", path.display()));
                }
                if content.contains("Monitor {name} desconectado.") {
                    errors.push(format!("{}: contains 'Monitor {{name}} desconectado.'", path.display()));
                }
                if content.contains("Layout de telas: modo {} aplicado.") {
                    errors.push(format!("{}: contains 'Layout de telas: modo {{}} aplicado.'", path.display()));
                }
                // Check if match app.lang is used in config_modal instead of message lookup
                if content.contains("Configurações & Preferências de Tema") {
                    errors.push(format!("{}: contains direct match instead of message lookup for Config Modal Title", path.display()));
                }
            }
        }
    }
}

#[test]
fn no_hardcoded_pt_strings_in_ui_and_backend() {
    let mut errors = Vec::new();
    check_dir(Path::new("src/ui"), &mut errors);
    check_dir(Path::new("src/backend"), &mut errors);
    
    if !errors.is_empty() {
        panic!("Found hardcoded strings or direct language matches instead of Messages struct:\n{}", errors.join("\n"));
    }
}
