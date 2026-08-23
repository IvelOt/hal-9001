//! Multi-boot leve embarcado (substitui o antigo instalador do Ventoy via
//! `scripts/ventoy.sh`).
//!
//! Em vez de invocar um instalador externo que reparticiona o pendrive,
//! `prepare_multiboot` apenas grava três arquivos numa partição FAT32 já
//! montada e gravável pelo usuário: um binário GRUB EFI (`EFI/BOOT/
//! BOOTX64.EFI`), um `grub.cfg` que escaneia `/ISOs/*.iso` em tempo de boot
//! (`boot/grub/grub.cfg`) e um arquivo-marcador (`ISOs/.hal9001-multiboot`)
//! usado por [`is_multiboot_installed`] para detectar se o drive já foi
//! preparado. Operação 100% não-destrutiva: nunca reparticiona, nunca
//! formata, nunca apaga nem sobrescreve ISOs já presentes em `/ISOs/`.
//!
//! Não requer privilégios elevados — a partição de dados já é montada e
//! gravável pelo usuário (é para isso que ela existe), então toda a
//! preparação é I/O de filesystem comum.

use std::path::{Path, PathBuf};

/// Nome do arquivo-marcador que identifica um drive já preparado para
/// multi-boot pelo HAL-9001 — vive dentro de `ISOs/`, nunca na raiz da
/// partição, para não colidir com o layout de um pendrive Ventoy real.
const MARKER_FILE: &str = ".hal9001-multiboot";
const ISOS_DIR: &str = "ISOs";

/// Bytes do binário GRUB EFI embarcado no binário do HAL-9001 em tempo de
/// compilação — ver `assets/multiboot/README.md` para o aviso sobre este ser
/// um placeholder que precisa ser substituído por um GRUB EFI real antes do
/// release.
const BOOTX64_EFI: &[u8] = include_bytes!("../../assets/multiboot/BOOTX64.EFI");
/// Conteúdo do `grub.cfg` embarcado — este, ao contrário do binário EFI, é
/// real e funcional (ver comentários no próprio arquivo).
const GRUB_CFG: &str = include_str!("../../assets/multiboot/grub.cfg");

/// Caminho absoluto de `<mount>/ISOs`.
fn isos_dir(mount_point: &Path) -> PathBuf {
    mount_point.join(ISOS_DIR)
}

/// Caminho absoluto do arquivo-marcador (`<mount>/ISOs/.hal9001-multiboot`).
fn marker_path(mount_point: &Path) -> PathBuf {
    isos_dir(mount_point).join(MARKER_FILE)
}

/// `true` quando `mount_point` (ponto de montagem já ativo de uma partição)
/// tem o arquivo-marcador do multi-boot HAL-9001 — usado pelo painel de
/// detalhes da aba Storage para exibir "Ativo (N ISOs)"/"Não instalado".
///
/// Baseado exclusivamente no arquivo-marcador (não no rótulo `Ventoy`/
/// `VTOYEFI`), então reconhece apenas drives preparados por
/// [`prepare_multiboot`] — um pendrive Ventoy "de verdade" (instalado pelo
/// instalador oficial, fora do HAL-9001) não é reportado como multi-boot
/// ativo por esta função, mesmo que suas ISOs continuem legíveis pelo
/// gerenciador de ISOs (ver `ventoy_data_partition`/`detect_ventoy` em
/// `backend::storage`, mantidos como detecção somente-leitura).
pub fn is_multiboot_installed(mount_point: &str) -> bool {
    marker_path(Path::new(mount_point)).is_file()
}

/// Conta arquivos `.iso`/`.img` (case-insensitive) diretamente sob
/// `<mount>/ISOs/` — devolve `0` quando o diretório não existe ainda (drive
/// nunca preparado, ou preparado mas sem nenhuma ISO copiada).
pub fn count_isos(mount_point: &str) -> usize {
    let dir = isos_dir(Path::new(mount_point));
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| crate::backend::storage::is_iso_or_img(&e.file_name().to_string_lossy()))
        .count()
}

/// Prepara `mount_point` (ponto de montagem já ativo de uma partição FAT32
/// gravável) para o multi-boot leve do HAL-9001:
///
/// 1. Cria `<mount>/ISOs/` se ainda não existir (nunca apaga o que já
///    houver dentro, caso já exista).
/// 2. Grava/sobrescreve `<mount>/EFI/BOOT/BOOTX64.EFI` e `<mount>/boot/grub/
///    grub.cfg` a partir dos bytes embarcados no binário — estes dois
///    arquivos são de propriedade do HAL-9001, então é seguro (e necessário
///    para atualizações) sobrescrevê-los a cada chamada.
/// 3. Grava o arquivo-marcador `<mount>/ISOs/.hal9001-multiboot` apenas se
///    ainda não existir — nunca trunca/recria um marcador já presente.
///
/// Idempotente: rodar novamente sobre um drive já preparado é um no-op do
/// ponto de vista do usuário (apenas re-grava os dois arquivos de boot, que
/// já não são visíveis/relevantes para o usuário final). Nunca toca em
/// nenhum outro arquivo dentro de `ISOs/` (as ISOs do usuário).
pub fn prepare_multiboot(mount_point: &Path) -> anyhow::Result<()> {
    let isos = isos_dir(mount_point);
    std::fs::create_dir_all(&isos)?;

    let efi_dir = mount_point.join("EFI").join("BOOT");
    std::fs::create_dir_all(&efi_dir)?;
    std::fs::write(efi_dir.join("BOOTX64.EFI"), BOOTX64_EFI)?;

    let grub_dir = mount_point.join("boot").join("grub");
    std::fs::create_dir_all(&grub_dir)?;
    std::fs::write(grub_dir.join("grub.cfg"), GRUB_CFG)?;

    let marker = marker_path(mount_point);
    if !marker.is_file() {
        std::fs::write(&marker, b"")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_mount() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn marker_absent_on_fresh_dir() {
        let dir = temp_mount();
        assert!(!is_multiboot_installed(dir.path().to_str().unwrap()));
    }

    #[test]
    fn marker_present_after_prepare() {
        let dir = temp_mount();
        prepare_multiboot(dir.path()).expect("prepare");
        assert!(is_multiboot_installed(dir.path().to_str().unwrap()));
        assert!(dir.path().join("EFI/BOOT/BOOTX64.EFI").is_file());
        assert!(dir.path().join("boot/grub/grub.cfg").is_file());
        assert!(dir.path().join("ISOs/.hal9001-multiboot").is_file());
    }

    #[test]
    fn count_isos_mixed_file_types() {
        let dir = temp_mount();
        let isos = dir.path().join("ISOs");
        std::fs::create_dir_all(&isos).unwrap();
        std::fs::write(isos.join("debian.iso"), b"x").unwrap();
        std::fs::write(isos.join("Fedora.ISO"), b"x").unwrap();
        std::fs::write(isos.join("rescue.img"), b"x").unwrap();
        std::fs::write(isos.join("notes.txt"), b"x").unwrap();
        std::fs::write(isos.join(".hal9001-multiboot"), b"").unwrap();
        std::fs::create_dir_all(isos.join("subdir.iso")).unwrap(); // dir, not a file
        assert_eq!(count_isos(dir.path().to_str().unwrap()), 3);
    }

    #[test]
    fn count_isos_missing_dir_is_zero() {
        let dir = temp_mount();
        assert_eq!(count_isos(dir.path().to_str().unwrap()), 0);
    }

    #[test]
    fn prepare_does_not_clobber_existing_isos() {
        let dir = temp_mount();
        let isos = dir.path().join("ISOs");
        std::fs::create_dir_all(&isos).unwrap();
        std::fs::write(isos.join("somefile.iso"), b"user-data-must-survive").unwrap();

        prepare_multiboot(dir.path()).expect("prepare");

        let content = std::fs::read(isos.join("somefile.iso")).unwrap();
        assert_eq!(content, b"user-data-must-survive");
        assert!(is_multiboot_installed(dir.path().to_str().unwrap()));
    }

    #[test]
    fn prepare_is_idempotent() {
        let dir = temp_mount();
        prepare_multiboot(dir.path()).expect("first prepare");
        // Simula um usuário criando uma ISO entre as duas chamadas.
        std::fs::write(dir.path().join("ISOs/my.iso"), b"data").unwrap();
        prepare_multiboot(dir.path()).expect("second prepare");

        assert!(is_multiboot_installed(dir.path().to_str().unwrap()));
        assert_eq!(count_isos(dir.path().to_str().unwrap()), 1);
        assert_eq!(
            std::fs::read(dir.path().join("ISOs/my.iso")).unwrap(),
            b"data"
        );
    }

    #[test]
    fn marker_path_is_under_isos_dir() {
        let mount = Path::new("/mnt/pendrive");
        assert_eq!(
            marker_path(mount),
            Path::new("/mnt/pendrive/ISOs/.hal9001-multiboot")
        );
    }
}
