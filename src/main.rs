#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }

// ================================================================
// PARAMÈTRES RÉSEAU — modifier avant compilation
// ================================================================
static WIFI_SSID:   &[u8] = b"a26nguep-hotspot";
static WIFI_PSK:    &[u8] = b"123456789";
static SERVER_IP:   &[u8] = b"10.42.0.1";
static DEVICE_NAME: &[u8] = b"heltec_v3";
static OS_NAME:     &[u8] = b"zephyr";

const SERVER_PORT:     u32 = 8080;
const NETWORK_TIMEOUT: u32 = 30;
const SOCKET_TIMEOUT:  u32 = 5;
const SEND_INTERVAL:   u32 = 5;   // secondes entre chaque envoi

// ================================================================
// HOST FUNCTIONS — pont WASM ↔ Zephyr C
// ================================================================
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
extern "C" {
    // Affichage
    fn host_print(msg_ptr: *const u8, msg_len: u32);

    // Wi-Fi / réseau
    fn host_wifi_connect(
        ssid_ptr: *const u8, ssid_len: u32,
        psk_ptr:  *const u8, psk_len:  u32,
    ) -> i32;
    fn host_wait_network_ready(timeout_secs: u32) -> i32;

    // GPIO
    fn host_gpio_blink();

    // TCP
    fn host_tcp_connect(
        ip_ptr: *const u8, ip_len: u32,
        port: u32, timeout_secs: u32,
    ) -> i32;
    fn host_tcp_send(fd: i32, buf_ptr: *const u8, buf_len: u32) -> i32;
    fn host_tcp_recv(fd: i32, buf_ptr: *mut u8,  buf_len: u32) -> i32;
    fn host_tcp_close(fd: i32);
    fn host_sleep(secs: u32);

    // ----------------------------------------------------------------
    // Métriques Zephyr OS (M1–M10)
    // ----------------------------------------------------------------

    /// M1 — CPU usage (%)
    /// Retourne un u32 dans [0, 100].
    /// Implémentation suggérée : mesure sur une fenêtre de 100 ms via
    /// k_thread_runtime_stats_get() + stats globales du scheduler.
    fn host_metric_cpu_usage() -> u32;

    /// M2 — Free heap (octets)
    /// sys_heap_runtime_stats_get() → free_bytes
    fn host_metric_free_heap() -> u32;

    /// M3 — Uptime (millisecondes)
    /// k_uptime_get() retourne un i64 ; on ne prend que les 32 bits bas.
    /// Suffisant pour ~49 jours de fonctionnement continu.
    fn host_metric_uptime_ms() -> u32;

    /// M4 — Bytes TX (octets cumulés depuis démarrage)
    /// net_stats_get() → net_stats.bytes.sent
    fn host_metric_bytes_tx() -> u32;

    /// M5 — Bytes RX (octets cumulés depuis démarrage)
    /// net_stats_get() → net_stats.bytes.received
    fn host_metric_bytes_rx() -> u32;

    /// M6 — Network errors (count cumulé)
    /// net_stats.ip_errors.protoerr + chkerr + ...
    fn host_metric_net_errors() -> u32;

    /// M7 — Stack usage du thread principal (%)
    /// k_thread_stack_space_get() → espace libre ; (total - libre) * 100 / total
    fn host_metric_stack_usage_pct() -> u32;

    /// M8 — Idle time ratio (%)
    /// Temps idle / uptime total × 100 ; mesuré via stats du thread idle Zephyr.
    fn host_metric_idle_ratio_pct() -> u32;

    /// M9 — RSSI Wi-Fi (dBm, valeur i32 négative typiquement -30 à -90)
    /// wifi_mgmt via NET_REQUEST_WIFI_IFACE_STATUS → status.rssi
    fn host_metric_rssi_dbm() -> i32;

    /// M10 — Reset count (compteur persistant en retained RAM ou NVS)
    /// hwinfo_get_reset_cause() + compteur NVS incrémenté au boot.
    fn host_metric_reset_count() -> u32;
}

// ================================================================
// BUFFERS STATIQUES
// Tous les buffers sont statiques pour éviter toute allocation heap.
// ================================================================
static mut TX_BUF:   [u8; 512] = [0u8; 512];
static mut RX_BUF:   [u8; 256] = [0u8; 256];
static mut JSON_BUF: [u8; 384] = [0u8; 384];
static mut LOG_BUF:  [u8; 128] = [0u8; 128];

// ================================================================
// UTILITAIRES no_std
// ================================================================

/// Copie src dans dst[offset..] et retourne le nouvel offset.
#[inline]
fn write_bytes(dst: &mut [u8], offset: usize, src: &[u8]) -> usize {
    let end = offset + src.len();
    dst[offset..end].copy_from_slice(src);
    end
}

/// Écrit n (u32) en décimal ASCII dans dst[offset..].
#[inline]
fn write_u32(dst: &mut [u8], offset: usize, mut n: u32) -> usize {
    if n == 0 {
        dst[offset] = b'0';
        return offset + 1;
    }
    let mut tmp = [0u8; 10];
    let mut len = 0usize;
    while n > 0 {
        tmp[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    for i in 0..len {
        dst[offset + i] = tmp[len - 1 - i];
    }
    offset + len
}

/// Écrit n (i32) en décimal ASCII signé dans dst[offset..].
/// Gère les valeurs négatives (ex : RSSI = -62 → b"-62").
#[inline]
fn write_i32(dst: &mut [u8], offset: usize, n: i32) -> usize {
    if n < 0 {
        dst[offset] = b'-';
        // Attention : i32::MIN ne peut pas être nié directement en i32.
        // On cast en i64 pour éviter l'overflow.
        write_u32(dst, offset + 1, (-(n as i64)) as u32)
    } else {
        write_u32(dst, offset, n as u32)
    }
}

// ================================================================
// LOG
// ================================================================
#[cfg(target_arch = "wasm32")]
#[inline]
fn log(msg: &[u8]) {
    unsafe { host_print(msg.as_ptr(), msg.len() as u32); }
}

#[cfg(target_arch = "wasm32")]
fn log_num(prefix: &[u8], n: u32) {
    let msg_len = unsafe {
        let buf: &mut [u8; 128] = &mut *(&raw mut LOG_BUF);
        let plen = prefix.len().min(110);
        buf[..plen].copy_from_slice(&prefix[..plen]);
        let mut i = plen;
        i = write_u32(buf, i, n);
        buf[i] = b'\n';
        i + 1
    };
    unsafe {
        let ptr = (&raw const LOG_BUF) as *const u8;
        host_print(ptr, msg_len as u32);
    }
}

// ================================================================
// COLLECTE DES MÉTRIQUES
// ================================================================

/// Struct légère pour transporter les 10 métriques.
/// Pas d'allocation heap : tous les champs sont des scalaires.
struct Metrics {
    cpu_usage_pct:   u32,   // M1
    free_heap_bytes: u32,   // M2
    uptime_ms:       u32,   // M3
    bytes_tx:        u32,   // M4
    bytes_rx:        u32,   // M5
    net_errors:      u32,   // M6
    stack_usage_pct: u32,   // M7
    idle_ratio_pct:  u32,   // M8
    rssi_dbm:        i32,   // M9  (valeur négative)
    reset_count:     u32,   // M10
}

#[cfg(target_arch = "wasm32")]
fn collect_metrics() -> Metrics {
    unsafe {
        Metrics {
            cpu_usage_pct:   host_metric_cpu_usage(),
            free_heap_bytes: host_metric_free_heap(),
            uptime_ms:       host_metric_uptime_ms(),
            bytes_tx:        host_metric_bytes_tx(),
            bytes_rx:        host_metric_bytes_rx(),
            net_errors:      host_metric_net_errors(),
            stack_usage_pct: host_metric_stack_usage_pct(),
            idle_ratio_pct:  host_metric_idle_ratio_pct(),
            rssi_dbm:        host_metric_rssi_dbm(),
            reset_count:     host_metric_reset_count(),
        }
    }
}

// ================================================================
// SÉRIALISATION JSON — entièrement dans JSON_BUF, pas d'alloc
//
// Format produit :
// {
//   "device":"heltec_v3","os":"zephyr",
//   "uptime_ms":45231,
//   "cpu_usage_pct":34,
//   "free_heap_bytes":42000,
//   "stack_usage_pct":28,
//   "idle_ratio_pct":66,
//   "bytes_tx":1240,
//   "bytes_rx":890,
//   "net_errors":0,
//   "rssi_dbm":-62,
//   "reset_count":0,
//   "status":"ok"
// }
// ================================================================
#[cfg(target_arch = "wasm32")]
fn build_json(m: &Metrics, seq: u32) -> usize {
    unsafe {
        let b: &mut [u8; 384] = &mut *(&raw mut JSON_BUF);
        let mut i = 0usize;

        // Ouverture
        i = write_bytes(b, i, b"{");

        // device
        i = write_bytes(b, i, b"\"device\":\"");
        i = write_bytes(b, i, DEVICE_NAME);
        i = write_bytes(b, i, b"\",");

        // os
        i = write_bytes(b, i, b"\"os\":\"");
        i = write_bytes(b, i, OS_NAME);
        i = write_bytes(b, i, b"\",");

        // seq (numéro de séquence)
        i = write_bytes(b, i, b"\"seq\":");
        i = write_u32(b, i, seq);
        i = write_bytes(b, i, b",");

        // M3 — uptime_ms
        i = write_bytes(b, i, b"\"uptime_ms\":");
        i = write_u32(b, i, m.uptime_ms);
        i = write_bytes(b, i, b",");

        // M1 — cpu_usage_pct
        i = write_bytes(b, i, b"\"cpu_usage_pct\":");
        i = write_u32(b, i, m.cpu_usage_pct);
        i = write_bytes(b, i, b",");

        // M2 — free_heap_bytes
        i = write_bytes(b, i, b"\"free_heap_bytes\":");
        i = write_u32(b, i, m.free_heap_bytes);
        i = write_bytes(b, i, b",");

        // M7 — stack_usage_pct
        i = write_bytes(b, i, b"\"stack_usage_pct\":");
        i = write_u32(b, i, m.stack_usage_pct);
        i = write_bytes(b, i, b",");

        // M8 — idle_ratio_pct
        i = write_bytes(b, i, b"\"idle_ratio_pct\":");
        i = write_u32(b, i, m.idle_ratio_pct);
        i = write_bytes(b, i, b",");

        // M4 — bytes_tx
        i = write_bytes(b, i, b"\"bytes_tx\":");
        i = write_u32(b, i, m.bytes_tx);
        i = write_bytes(b, i, b",");

        // M5 — bytes_rx
        i = write_bytes(b, i, b"\"bytes_rx\":");
        i = write_u32(b, i, m.bytes_rx);
        i = write_bytes(b, i, b",");

        // M6 — net_errors
        i = write_bytes(b, i, b"\"net_errors\":");
        i = write_u32(b, i, m.net_errors);
        i = write_bytes(b, i, b",");

        // M9 — rssi_dbm (i32 signé)
        i = write_bytes(b, i, b"\"rssi_dbm\":");
        i = write_i32(b, i, m.rssi_dbm);
        i = write_bytes(b, i, b",");

        // M10 — reset_count
        i = write_bytes(b, i, b"\"reset_count\":");
        i = write_u32(b, i, m.reset_count);
        i = write_bytes(b, i, b",");

        // status — dérivé des métriques
        let status: &[u8] = derive_status(m);
        i = write_bytes(b, i, b"\"status\":\"");
        i = write_bytes(b, i, status);
        i = write_bytes(b, i, b"\"}");

        i
    }
}

/// Dérive le champ `status` depuis les métriques brutes.
/// Priorité : cpu_saturated > heap_low > net_degraded > ok
fn derive_status(m: &Metrics) -> &'static [u8] {
    if m.cpu_usage_pct > 80           { return b"cpu_saturated"; }
    if m.free_heap_bytes < 4096       { return b"heap_low"; }      // < 4 KB
    if m.net_errors > 3               { return b"net_degraded"; }
    if m.stack_usage_pct > 85         { return b"stack_overflow_risk"; }
    b"ok"
}

// ================================================================
// ENVOI HTTP POST avec les métriques
// ================================================================
#[cfg(target_arch = "wasm32")]
fn send_metrics(seq: u32) {
    log_num(b"[METRICS] collecting seq=", seq);

    // 1. Collecter les métriques
    let m = collect_metrics();

    // Log résumé
    log_num(b"  cpu=",         m.cpu_usage_pct);
    log_num(b"  heap_free=",   m.free_heap_bytes);
    log_num(b"  uptime_ms=",   m.uptime_ms);
    log_num(b"  stack_pct=",   m.stack_usage_pct);
    log_num(b"  idle_pct=",    m.idle_ratio_pct);
    log_num(b"  net_errors=",  m.net_errors);
    log_num(b"  reset_cnt=",   m.reset_count);

    // 2. Sérialiser en JSON
    let json_len = build_json(&m, seq);

    // 3. Construire la requête HTTP dans TX_BUF
    let tx_len = unsafe {
        let tx: &mut [u8; 512] = &mut *(&raw mut TX_BUF);
        let mut j = 0;
        j = write_bytes(tx, j, b"POST /metrics HTTP/1.0\r\nHost: ");
        j = write_bytes(tx, j, SERVER_IP);
        j = write_bytes(tx, j, b":");
        j = write_u32(tx, j, SERVER_PORT);
        j = write_bytes(tx, j,
            b"\r\nContent-Type: application/json\r\nContent-Length: ");
        j = write_u32(tx, j, json_len as u32);
        j = write_bytes(tx, j, b"\r\nConnection: close\r\n\r\n");
        // Copie du JSON depuis JSON_BUF
        let json_slice: &[u8; 384] = &*(&raw const JSON_BUF);
        j = write_bytes(tx, j, &json_slice[..json_len]);
        j
    };

    // 4. Connexion TCP
    let fd = unsafe {
        host_tcp_connect(
            SERVER_IP.as_ptr(), SERVER_IP.len() as u32,
            SERVER_PORT, SOCKET_TIMEOUT,
        )
    };
    if fd < 0 {
        log(b"[METRICS] TCP connect failed\n");
        return;
    }
    log(b"[METRICS] TCP connected\n");

    // 5. Envoi
    let sent = unsafe {
        let tx_ptr = (&raw const TX_BUF) as *const u8;
        host_tcp_send(fd, tx_ptr, tx_len as u32)
    };

    if sent > 0 {
        log(b"[METRICS] request sent, waiting ACK...\n");
        let received = unsafe {
            let rx_ptr = (&raw mut RX_BUF) as *mut u8;
            host_tcp_recv(fd, rx_ptr, (256 - 1) as u32)
        };
        if received > 0 {
            log(b"[METRICS] ACK received\n");
        } else {
            log(b"[METRICS] no ACK\n");
        }
    } else {
        log(b"[METRICS] send failed\n");
    }

    unsafe { host_tcp_close(fd); }
    log(b"[METRICS] socket closed\n");
}

// ================================================================
// POINT D'ENTRÉE WASM
// ================================================================
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn main() {
    log(b"============================================\n");
    log(b" WASM Metrics Collector - Zephyr OS\n");
    log(b"============================================\n");
    log(b"Metriques : CPU / Heap / Uptime / TX / RX /\n");
    log(b"            Errors / Stack / Idle / RSSI / Resets\n");
    log(b"--------------------------------------------\n");
    log(b"Connexion Wi-Fi...\n");

    let ret = unsafe {
        host_wifi_connect(
            WIFI_SSID.as_ptr(), WIFI_SSID.len() as u32,
            WIFI_PSK.as_ptr(),  WIFI_PSK.len()  as u32,
        )
    };
    if ret != 0 {
        log(b"[ERR] wifi_connect failed\n");
        return;
    }

    log(b"Attente IP DHCP...\n");
    let ret = unsafe { host_wait_network_ready(NETWORK_TIMEOUT) };
    if ret != 0 {
        log(b"[ERR] DHCP timeout\n");
        return;
    }
    log(b"Reseau pret\n");

    let mut seq: u32 = 0;
    loop {
        seq += 1;
        // Clignote la LED à chaque collecte (M1–M10 + envoi)
        unsafe { host_gpio_blink(); }
        // Collecter et envoyer les métriques
        send_metrics(seq);
        // Attendre avant la prochaine collecte
        unsafe { host_sleep(SEND_INTERVAL); }
    }
}

// ================================================================
// STUB x86_64 — pour rust-analyzer et cargo check
// ================================================================
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("Ce binaire est concu pour wasm32-unknown-unknown.");
    eprintln!("Compiler avec : bash build_wasm.sh");
}