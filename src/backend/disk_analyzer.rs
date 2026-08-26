//! Analisador de Espaço em Disco Nativo (estilo `ncdu`/`dua`), 100% Rust puro
//! — varredura recursiva via `std::fs::read_dir`, sem nenhuma dependência
//! externa. Sobe uma linha (arquivo ou subdiretório) por vez, somando o
//! tamanho recursivo de cada subdiretório para permitir a navegação "entrar/
//! voltar" da UI (ver `ui::storage::draw_analyzer`).

use std::path::{Path, PathBuf};

use crate::events::{AppEvent, EventTx};

/// Uma linha da listagem do Analisador: um arquivo, ou um subdiretório com o
/// tamanho de toda a sua árvore já somado.
#[derive(Debug, Clone, PartialEq)]
pub struct DiskUsageItem {
    pub name: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    /// Percentual (0.0..=100.0) de `size_bytes` em relação ao maior item da
    /// listagem — usado para desenhar a barra `[████░░░░]`.
    pub percentage: f32,
}

/// Resultado completo (concluído) de uma varredura de `current_path`.
#[derive(Debug, Clone, PartialEq)]
pub struct DiskUsageSnapshot {
    pub current_path: PathBuf,
    /// Soma dos tamanhos de todos os itens listados (não o tamanho do
    /// filesystem inteiro — apenas o que foi possível somar em `current_path`).
    pub total_bytes: u64,
    pub items: Vec<DiskUsageItem>,
    pub is_scanning: bool,
}

/// Soma recursiva do tamanho de `path` (arquivos regulares) — símbolos
/// (symlinks) contam apenas seu próprio tamanho, sem seguir o alvo, para
/// nunca entrar em ciclo. Diretórios ilegíveis (permissão negada, etc.)
/// contribuem com `0` em vez de abortar a varredura inteira.
fn dir_size_recursive(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            total += dir_size_recursive(&entry.path());
        } else {
            total += meta.len();
        }
    }
    total
}

/// Varre um único nível de `path`, somando recursivamente o tamanho de cada
/// subdiretório — o núcleo síncrono/bloqueante da varredura, chamado dentro
/// de `tokio::task::spawn_blocking` por [`scan`].
pub fn scan_dir(path: &Path) -> std::io::Result<DiskUsageSnapshot> {
    let entries = std::fs::read_dir(path)?;

    let mut items: Vec<DiskUsageItem> = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let (is_dir, size_bytes) = if meta.is_dir() {
            (true, dir_size_recursive(&entry.path()))
        } else {
            (false, meta.len())
        };
        items.push(DiskUsageItem {
            name,
            is_dir,
            size_bytes,
            percentage: 0.0,
        });
    }

    items.sort_by_key(|i| std::cmp::Reverse(i.size_bytes));

    let max_size = items.iter().map(|i| i.size_bytes).max().unwrap_or(0);
    if max_size > 0 {
        for item in &mut items {
            item.percentage = (item.size_bytes as f64 / max_size as f64 * 100.0) as f32;
        }
    }

    let total_bytes = items.iter().map(|i| i.size_bytes).sum();

    Ok(DiskUsageSnapshot {
        current_path: path.to_path_buf(),
        total_bytes,
        items,
        is_scanning: false,
    })
}

/// Varre `path` numa `spawn_blocking` (a recursão em `std::fs` bloqueia a
/// thread) e publica o resultado — `StorageAnalyzerSnapshot` em caso de
/// sucesso, `StorageAnalyzerError` caso `path` não possa ser listado.
pub async fn scan(path: PathBuf, tx: EventTx) {
    let result = tokio::task::spawn_blocking(move || scan_dir(&path)).await;
    match result {
        Ok(Ok(snapshot)) => {
            let _ = tx.send(AppEvent::StorageAnalyzerSnapshot(Box::new(snapshot)));
        }
        Ok(Err(e)) => {
            let _ = tx.send(AppEvent::StorageAnalyzerError {
                path: PathBuf::new(),
                message: e.to_string(),
            });
        }
        Err(e) => {
            let _ = tx.send(AppEvent::StorageAnalyzerError {
                path: PathBuf::new(),
                message: e.to_string(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_files_and_dirs_sorted_by_size_desc() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("small.txt"), vec![0u8; 10]).unwrap();
        std::fs::write(tmp.path().join("big.txt"), vec![0u8; 1000]).unwrap();
        std::fs::create_dir(tmp.path().join("subdir")).unwrap();
        std::fs::write(tmp.path().join("subdir/nested.txt"), vec![0u8; 500]).unwrap();

        let snap = scan_dir(tmp.path()).unwrap();
        assert_eq!(snap.items.len(), 3);
        assert_eq!(snap.items[0].name, "big.txt");
        assert_eq!(snap.items[0].size_bytes, 1000);
        assert_eq!(snap.items[0].percentage, 100.0);

        let subdir = snap.items.iter().find(|i| i.name == "subdir").unwrap();
        assert!(subdir.is_dir);
        assert_eq!(subdir.size_bytes, 500);

        assert_eq!(snap.total_bytes, 1510);
        assert!(!snap.is_scanning);
    }

    #[test]
    fn empty_dir_yields_no_items() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = scan_dir(tmp.path()).unwrap();
        assert!(snap.items.is_empty());
        assert_eq!(snap.total_bytes, 0);
    }

    #[test]
    fn missing_dir_errors() {
        assert!(scan_dir(Path::new("/nonexistent/path/for/hal9001/tests")).is_err());
    }
}
