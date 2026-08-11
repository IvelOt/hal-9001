mod ai_agent;
mod backend;
mod events;
mod ui;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    println!("HAL-9001 — Central TUI de controle de sistema");
    println!("Inicializando System Interactivity Layer...\n");

    match backend::power::Power::new().await {
        Ok(power) => {
            match power.on_battery().await {
                Ok(on_battery) => println!("[power] em bateria: {on_battery}"),
                Err(e) => println!("[power] erro ao consultar OnBattery: {e}"),
            }
            match power.primary_battery().await {
                Ok(Some(battery)) => {
                    println!(
                        "[power] bateria principal: estado={:?} percentual={:.1}% capacidade={:.1}%",
                        battery.state, battery.percentage, battery.capacity
                    );
                    if let Some(time) = battery.estimated_time_remaining() {
                        println!("[power] tempo restante estimado: {time:?}");
                    }
                }
                Ok(None) => println!("[power] nenhuma bateria principal presente"),
                Err(e) => println!("[power] erro ao consultar bateria principal: {e}"),
            }
            match power.batteries().await {
                Ok(batteries) => println!("[power] {} bateria(s) no total", batteries.len()),
                Err(e) => println!("[power] erro ao enumerar baterias: {e}"),
            }
        }
        Err(e) => println!("[power] indisponível (UPower D-Bus): {e}"),
    }

    match backend::storage::Storage::new().await {
        Ok(storage) => match storage.block_devices().await {
            Ok(devices) => {
                println!("[storage] {} dispositivo(s) removível(is):", devices.len());
                for d in &devices {
                    let mounted = storage
                        .is_mounted(&d.object_path)
                        .await
                        .unwrap_or(false);
                    println!(
                        "  - {} rótulo={:?} uuid={:?} tamanho={} bytes montado={mounted}",
                        d.device, d.label, d.uuid, d.size
                    );
                }
            }
            Err(e) => println!("[storage] erro ao listar dispositivos: {e}"),
        },
        Err(e) => println!("[storage] indisponível (UDisks2 D-Bus): {e}"),
    }

    match backend::controls::Controls::new().await {
        Ok(controls) => {
            match controls.get_volume().await {
                Ok(volume) => println!("[controls] volume do sink padrão: {:.1}%", volume * 100.0),
                Err(e) => println!("[controls] erro ao ler volume (wpctl): {e}"),
            }
            match controls.get_brightness_percent().await {
                Ok(brightness) => println!("[controls] brilho: {brightness}%"),
                Err(e) => println!("[controls] erro ao ler brilho (brightnessctl): {e}"),
            }
        }
        Err(e) => println!("[controls] indisponível (wpctl/brightnessctl): {e}"),
    }

    match backend::bluetooth::Bluetooth::new().await {
        Ok(bluetooth) => match bluetooth.adapter_paths().await {
            Ok(adapters) => {
                println!("[bluetooth] {} adaptador(es): {:?}", adapters.len(), adapters);
                match bluetooth.devices().await {
                    Ok(devices) => {
                        println!("[bluetooth] {} dispositivo(s) conhecido(s):", devices.len());
                        for device in &devices {
                            println!(
                                "  - {} emparelhado={} conectado={}",
                                device.name, device.paired, device.connected
                            );
                        }
                    }
                    Err(e) => println!("[bluetooth] erro ao listar dispositivos: {e}"),
                }
            }
            Err(e) => println!("[bluetooth] erro ao listar adaptadores: {e}"),
        },
        Err(e) => println!("[bluetooth] indisponível (BlueZ D-Bus): {e}"),
    }

    match backend::network::Network::new().await {
        Ok(network) => {
            match network.active_wifi().await {
                Ok(Some(wifi)) => println!(
                    "[network] wi-fi ativo: ssid={:?} sinal={}% dispositivo={}",
                    wifi.ssid, wifi.strength, wifi.device_path
                ),
                Ok(None) => println!("[network] nenhuma rede sem fio ativa"),
                Err(e) => println!("[network] erro ao consultar wi-fi ativo: {e}"),
            }
            match network.wireless_enabled().await {
                Ok(enabled) => println!("[network] wireless_enabled={enabled}"),
                Err(e) => println!("[network] erro ao ler wireless_enabled: {e}"),
            }
        }
        Err(e) => println!("[network] indisponível (NetworkManager D-Bus): {e}"),
    }

    Ok(())
}
