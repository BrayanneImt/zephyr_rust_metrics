# rust_metrics - Collecteur de métriques IoT (no_std, WASM)

Collecte et envoi périodique des 10 métriques système Zephyr OS
vers un serveur HTTP, via un module WebAssembly exécuté par WAMR.

## Métriques collectées

| # | Champ JSON | Unité | Source Zephyr |
|---|---|---|---|
| M1 | `cpu_usage_pct` | % | `k_thread_runtime_stats_all_get()` |
| M2 | `free_heap_bytes` | octets | `sys_heap_runtime_stats_get()` |
| M3 | `uptime_ms` | ms | `k_uptime_get()` |
| M4 | `bytes_tx` | octets | `net_stats.bytes.sent` |
| M5 | `bytes_rx` | octets | `net_stats.bytes.received` |
| M6 | `net_errors` | count | `net_stats.ip_errors` |
| M7 | `stack_usage_pct` | % | `k_thread_stack_space_get()` |
| M8 | `idle_ratio_pct` | % | `k_thread_runtime_stats_all_get()` (idle_cycles) |
| M9 | `rssi_dbm` | dBm | `NET_REQUEST_WIFI_IFACE_STATUS` |
| M10 | `reset_count` | count | `hwinfo_get_reset_cause()` |

## Exemple de payload envoyé

```json
{
  "device": "heltec_v3",
  "os": "zephyr",
  "seq": 1,
  "uptime_ms": 45231,
  "cpu_usage_pct": 34,
  "free_heap_bytes": 42000,
  "stack_usage_pct": 28,
  "idle_ratio_pct": 66,
  "bytes_tx": 1240,
  "bytes_rx": 890,
  "net_errors": 0,
  "rssi_dbm": -62,
  "reset_count": 0,
  "status": "ok"
}
```

Le champ `status` est dérivé automatiquement :

- `"ok"` — tout est nominal
- `"cpu_saturated"` — CPU > 80%
- `"heap_low"` — heap libre < 4 KB
- `"net_degraded"` — erreurs réseau > 3
- `"stack_overflow_risk"` — stack > 85%

## Préréquis

```bash
# Installer Rust (si absent)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Ajouter la cible WebAssembly WASI
rustup target add wasm32-wasip1

# Vérifier
rustup target list --installed | grep wasm
# → wasm32-wasip1
```

## Structure du projet

```text
rust_http/
├── src/
│   └── main.rs       ← Application principale (HTTP TCP + LED blinky)
├── Cargo.toml        ← Manifeste du projet
├── build_wasm.sh     ← Script de compilation Rust → WASM
└── upload.py         ← Script d'envoi UART vers l'équipement IoT
```

## Configuration du projet

```bash
#cloner le repo
git clone https://github.com/BrayanneImt/zephyr_rust_metrics.git
```

## Paramètres à adapter avant compilation

Dans `src/main.rs`, modifier si nécessaire :

```c
const WIFI_SSID: &str = "wifi_name";   // Nom du hotspot
const WIFI_PSK:  &str = "password";            // Mot de passe
const SERVER_IP: &str = "IP_server";        // IP du PC sur le hotspot
const SERVER_PORT: u16 = 8080;
```

## execution du projet pour la generation du fichier wasm

```bash
cd rust_http
bash build_wasm.sh

# Ou manuellement :
cargo build --target wasm32-wasip1 --release

# Copier le résultat
cp target/wasm32-wasip1/release/http_wasm.wasm http_rust.wasm
```

## test sur PC des executable wasm avant deploiement

```bash
# Installer wasmtime
curl https://wasmtime.dev/install.sh -sSf | bash
source ~/.bashrc

# Tester le WASM Rust
wasmtime http_rust.wasm
```

## Déploiement sur l'équipement IoT

```bash
python3 -m venv .venv

# activer l'environnment virtuel
source .venv/bin/activate
# sous windows
source mon_env/bin/activate

# Installer les dependences necessaire
pip install pyserial

# Execution
python3 upload.py
# → Progression de l'upload affichée
# → UPLOAD DONE
```
