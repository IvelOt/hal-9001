//! Bluetooth (BlueZ D-Bus) — descoberta, pareamento e conexão.
//!
//! Conforme seção 1.1 de `docs/backend_architecture.md`. Usa o daemon
//! `org.bluez` para iniciar/parar varredura, capturar dispositivos
//! descobertos via sinal `InterfacesAdded` e conectar/desconectar
//! dispositivos (`org.bluez.Device1`).

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use zbus::fdo::{InterfacesAdded, ObjectManagerProxy};
use zbus::names::OwnedInterfaceName;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{proxy, Connection};

/// Interface D-Bus de um dispositivo Bluetooth.
const BLUEZ_DEVICE_IFACE: &str = "org.bluez.Device1";
/// Interface D-Bus de um adaptador Bluetooth.
const BLUEZ_ADAPTER_IFACE: &str = "org.bluez.Adapter1";

/// Representa um dispositivo Bluetooth (descoberto ou já conhecido).
#[derive(Debug, Clone, Serialize)]
pub struct BluetoothDevice {
    /// Caminho de objeto D-Bus (ex.: `/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF`).
    pub object_path: String,
    /// Nome amigável do dispositivo (vazio enquanto não resolvido).
    pub name: String,
    /// `true` se o dispositivo está emparelhado com o adaptador.
    pub paired: bool,
    /// `true` se há uma conexão ativa com o dispositivo.
    pub connected: bool,
}

/// Proxy para a interface `org.bluez.Adapter1` (varredura).
#[proxy(
    interface = "org.bluez.Adapter1",
    default_service = "org.bluez"
)]
trait Adapter1 {
    /// Inicia a varredura ativa por dispositivos.
    fn start_discovery(&self) -> zbus::Result<()>;

    /// Encerra a varredura para economizar recursos e energia.
    fn stop_discovery(&self) -> zbus::Result<()>;
}

/// Proxy para a interface `org.bluez.Device1` (conexão/propriedades).
#[proxy(
    interface = "org.bluez.Device1",
    default_service = "org.bluez"
)]
trait Device1 {
    /// Estabelece a conexão com o dispositivo.
    fn connect(&self) -> zbus::Result<()>;

    /// Encerra a conexão com o dispositivo.
    fn disconnect(&self) -> zbus::Result<()>;

    /// Nome amigável do dispositivo.
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    /// `true` se o dispositivo está emparelhado.
    #[zbus(property)]
    fn paired(&self) -> zbus::Result<bool>;

    /// `true` se há conexão ativa.
    #[zbus(property)]
    fn connected(&self) -> zbus::Result<bool>;
}

/// Backend Bluetooth que encapsula a conexão D-Bus ao BlueZ.
pub struct Bluetooth {
    connection: Connection,
}

impl Bluetooth {
    /// Abre uma conexão com o barramento do sistema e retorna o backend Bluetooth.
    pub async fn new() -> Result<Self> {
        let connection = Connection::system()
            .await
            .context("falha ao conectar ao barramento D-Bus do sistema")?;
        Ok(Self { connection })
    }

    /// Lista os caminhos de objeto de todos os adaptadores Bluetooth disponíveis.
    pub async fn adapter_paths(&self) -> Result<Vec<String>> {
        let objects = self
            .managed_objects()
            .await
            .context("falha ao consultar objetos do BlueZ")?;

        Ok(objects
            .iter()
            .filter_map(|(path, ifaces)| {
                if ifaces.contains_key(BLUEZ_ADAPTER_IFACE) {
                    Some(path.to_string())
                } else {
                    None
                }
            })
            .collect())
    }

    /// Lista todos os dispositivos Bluetooth conhecidos (emparelhados ou não).
    pub async fn devices(&self) -> Result<Vec<BluetoothDevice>> {
        let objects = self
            .managed_objects()
            .await
            .context("falha ao consultar objetos do BlueZ")?;

        let mut devices = Vec::new();
        for (path, ifaces) in &objects {
            if let Some(device) = device_from_managed_objects(ifaces, path.as_str()) {
                devices.push(device);
            }
        }
        Ok(devices)
    }

    /// Inicia a varredura ativa no primeiro adaptador disponível.
    #[allow(dead_code)]
    pub async fn start_discovery(&self) -> Result<()> {
        let adapter = self.adapter_proxy().await?;
        adapter
            .start_discovery()
            .await
            .context("falha ao chamar BlueZ.Adapter1.StartDiscovery")
    }

    /// Encerra a varredura ativa no primeiro adaptador disponível.
    #[allow(dead_code)]
    pub async fn stop_discovery(&self) -> Result<()> {
        let adapter = self.adapter_proxy().await?;
        adapter
            .stop_discovery()
            .await
            .context("falha ao chamar BlueZ.Adapter1.StopDiscovery")
    }

    /// Escuta o sinal `InterfacesAdded` durante `timeout` enquanto mantém a
    /// varredura ativa, coletando os novos dispositivos descobertos.
    ///
    /// A varredura é iniciada automaticamente e interrompida ao final.
    #[allow(dead_code)]
    pub async fn discover_devices(&self, timeout: Duration) -> Result<Vec<BluetoothDevice>> {
        let manager = ObjectManagerProxy::builder(&self.connection)
            .destination("org.bluez")?
            .path("/")?
            .build()
            .await
            .context("falha ao criar proxy ObjectManager do BlueZ")?;
        let mut stream = manager
            .receive_interfaces_added()
            .await
            .context("falha ao registrar listener do sinal InterfacesAdded")?;

        self.start_discovery().await?;

        let deadline = tokio::time::Instant::now() + timeout;
        let mut devices = Vec::new();
        while let Ok(Some(message)) = tokio::time::timeout_at(deadline, stream.next()).await {
            let Some(signal) = InterfacesAdded::from_message(message) else {
                continue;
            };
            let args = signal
                .args()
                .context("falha ao ler argumentos do sinal InterfacesAdded")?;
            if let Some(device) = device_from_interfaces_added(&args) {
                if !devices
                    .iter()
                    .any(|d: &BluetoothDevice| d.object_path == device.object_path)
                {
                    devices.push(device);
                }
            }
        }

        let _ = self.stop_discovery().await;
        Ok(devices)
    }

    /// Estabelece a conexão com o dispositivo apontado por `object_path`.
    ///
    /// Ação mutável — será acionada pela TUI / Gatekeeper de consentimento.
    #[allow(dead_code)]
    pub async fn connect_device(&self, object_path: &str) -> Result<()> {
        self.device_proxy(object_path).await?.connect().await.context(
            "falha ao conectar dispositivo Bluetooth via BlueZ.Device1.Connect",
        )
    }

    /// Encerra a conexão com o dispositivo apontado por `object_path`.
    ///
    /// Ação mutável — será acionada pela TUI / Gatekeeper de consentimento.
    #[allow(dead_code)]
    pub async fn disconnect_device(&self, object_path: &str) -> Result<()> {
        self.device_proxy(object_path)
            .await?
            .disconnect()
            .await
            .context("falha ao desconectar dispositivo Bluetooth via BlueZ.Device1.Disconnect")
    }

    /// Consulta as propriedades atuais de um dispositivo já conhecido.
    #[allow(dead_code)]
    pub async fn device_info(&self, object_path: &str) -> Result<BluetoothDevice> {
        let device = self.device_proxy(object_path).await?;
        Ok(BluetoothDevice {
            object_path: object_path.to_string(),
            name: device.name().await.unwrap_or_default(),
            paired: device.paired().await.unwrap_or(false),
            connected: device.connected().await.unwrap_or(false),
        })
    }

    /// Consulta todos os objetos gerenciados pelo BlueZ (ObjectManager em `/`).
    async fn managed_objects(
        &self,
    ) -> Result<HashMap<OwnedObjectPath, HashMap<OwnedInterfaceName, HashMap<String, OwnedValue>>>>
    {
        let manager = ObjectManagerProxy::builder(&self.connection)
            .destination("org.bluez")?
            .path("/")?
            .build()
            .await
            .context("falha ao criar proxy ObjectManager do BlueZ")?;
        manager
            .get_managed_objects()
            .await
            .context("falha ao chamar BlueZ.GetManagedObjects")
    }

    /// Cria um proxy `Adapter1` para o primeiro adaptador encontrado.
    async fn adapter_proxy(&self) -> Result<Adapter1Proxy<'_>> {
        let path = self
            .adapter_paths()
            .await?
            .into_iter()
            .next()
            .context("nenhum adaptador Bluetooth encontrado")?;
        let object_path: OwnedObjectPath = path
            .try_into()
            .context("caminho de objeto D-Bus de adaptador inválido")?;
        Adapter1Proxy::new(&self.connection, object_path)
            .await
            .context("falha ao criar proxy Adapter1 do BlueZ")
    }

    /// Cria um proxy `Device1` para o caminho de objeto informado.
    async fn device_proxy(&self, object_path: &str) -> Result<Device1Proxy<'_>> {
        let object_path: OwnedObjectPath = object_path
            .try_into()
            .with_context(|| format!("caminho de objeto D-Bus inválido: {object_path}"))?;
        Device1Proxy::new(&self.connection, object_path)
            .await
            .context("falha ao criar proxy Device1 do BlueZ")
    }
}

/// Abstração para valores de propriedade D-Bus (emprestados ou próprios).
trait PropertyValue {
    fn get_str(&self) -> Option<String>;
    fn get_bool(&self) -> Option<bool>;
}

impl PropertyValue for Value<'_> {
    fn get_str(&self) -> Option<String> {
        self.downcast_ref::<String>().ok()
    }
    fn get_bool(&self) -> Option<bool> {
        self.downcast_ref::<bool>().ok()
    }
}

impl PropertyValue for OwnedValue {
    fn get_str(&self) -> Option<String> {
        self.downcast_ref::<String>().ok()
    }
    fn get_bool(&self) -> Option<bool> {
        self.downcast_ref::<bool>().ok()
    }
}

/// Lê uma propriedade de texto de um mapa de propriedades D-Bus.
fn prop_str<K, V>(props: &HashMap<K, V>, key: &str) -> String
where
    K: std::borrow::Borrow<str> + Eq + std::hash::Hash,
    V: PropertyValue,
{
    props.get(key).and_then(PropertyValue::get_str).unwrap_or_default()
}

/// Lê uma propriedade booleana de um mapa de propriedades D-Bus.
fn prop_bool<K, V>(props: &HashMap<K, V>, key: &str) -> bool
where
    K: std::borrow::Borrow<str> + Eq + std::hash::Hash,
    V: PropertyValue,
{
    props.get(key).and_then(PropertyValue::get_bool).unwrap_or(false)
}

/// Extrai um [`BluetoothDevice`] do mapa de interfaces de `GetManagedObjects`.
fn device_from_managed_objects(
    ifaces: &HashMap<OwnedInterfaceName, HashMap<String, OwnedValue>>,
    object_path: &str,
) -> Option<BluetoothDevice> {
    let props = ifaces.get(BLUEZ_DEVICE_IFACE)?;
    Some(BluetoothDevice {
        object_path: object_path.to_string(),
        name: prop_str(props, "Name"),
        paired: prop_bool(props, "Paired"),
        connected: prop_bool(props, "Connected"),
    })
}

/// Extrai um [`BluetoothDevice`] dos argumentos do sinal `InterfacesAdded`.
#[allow(dead_code)]
fn device_from_interfaces_added(args: &zbus::fdo::InterfacesAddedArgs<'_>) -> Option<BluetoothDevice> {
    let props = args.interfaces_and_properties.get(BLUEZ_DEVICE_IFACE)?;
    Some(BluetoothDevice {
        object_path: args.object_path.as_str().to_string(),
        name: prop_str(props, "Name"),
        paired: prop_bool(props, "Paired"),
        connected: prop_bool(props, "Connected"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn managed_device_ifaces() -> HashMap<OwnedInterfaceName, HashMap<String, OwnedValue>> {
        let mut props = HashMap::new();
        props.insert(
            "Name".to_string(),
            OwnedValue::try_from(Value::from("Fone de Ouvido".to_string())).unwrap(),
        );
        props.insert("Paired".to_string(), OwnedValue::from(true));
        props.insert("Connected".to_string(), OwnedValue::from(false));

        let mut ifaces: HashMap<OwnedInterfaceName, HashMap<String, OwnedValue>> = HashMap::new();
        ifaces.insert(OwnedInterfaceName::try_from("org.bluez.Device1").unwrap(), props);
        ifaces
    }

    #[test]
    fn extracts_device_from_managed_objects() {
        let device =
            device_from_managed_objects(&managed_device_ifaces(), "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF")
                .expect("objeto Device1 deveria ser extraído");

        assert_eq!(device.object_path, "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF");
        assert_eq!(device.name, "Fone de Ouvido");
        assert!(device.paired);
        assert!(!device.connected);
    }

    #[test]
    fn ignores_objects_without_device1_interface() {
        let ifaces: HashMap<OwnedInterfaceName, HashMap<String, OwnedValue>> = HashMap::new();
        assert!(device_from_managed_objects(&ifaces, "/org/bluez/hci0").is_none());
    }

    #[test]
    fn defaults_used_when_properties_are_missing() {
        let mut ifaces = managed_device_ifaces();
        ifaces
            .get_mut(&OwnedInterfaceName::try_from("org.bluez.Device1").unwrap())
            .unwrap()
            .clear();

        let device = device_from_managed_objects(&ifaces, "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF")
            .expect("objeto Device1 deveria ser extraído mesmo sem propriedades");
        assert_eq!(device.name, "");
        assert!(!device.paired);
        assert!(!device.connected);
    }
}
