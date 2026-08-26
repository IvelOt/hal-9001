{ lib
, rustPlatform
, pkg-config
, dbus
, systemd
, makeWrapper
, networkmanager ? null
, bluez ? null
, udisks2 ? null
, upower ? null
, wireplumber ? null
, pulseaudio ? null
, xorg ? null
, wlr-randr ? null
, yazi ? null
}:

rustPlatform.buildRustPackage {
  pname = "hall-9001";
  version = "0.1.0";

  src = lib.cleanSource ./.;

  cargoLock = {
    lockFile = ./Cargo.lock;
  };

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];

  buildInputs = [
    dbus
    systemd
  ];

  postInstall = ''
    # Atalho alternativo
    ln -sf $out/bin/hal9001 $out/bin/hall-9001

    # Desktop Entry
    install -Dm644 packaging/desktop/hall-9001.desktop $out/share/applications/hall-9001.desktop

    # Exemplo de configuração e documentação
    install -Dm644 config.toml $out/share/doc/hall-9001/config.toml.example
    install -Dm644 README.md $out/share/doc/hall-9001/README.md

    # Wrapper opcional garantindo acesso aos utilitários recomendados
    wrapProgram $out/bin/hal9001 \
      --prefix PATH : ${lib.makeBinPath (lib.filter (p: p != null) [
        networkmanager
        bluez
        udisks2
        upower
        wireplumber
        pulseaudio
        (if xorg != null then xorg.xrandr else null)
        wlr-randr
        yazi
      ])}
  '';

  meta = with lib; {
    description = "Central TUI de Controle do Sistema & Assistente de Sistema (Rust/ratatui)";
    homepage = "https://github.com/IvelOt/hall-9001";
    license = licenses.mit;
    maintainers = [ ];
    mainProgram = "hal9001";
    platforms = platforms.linux;
  };
}
