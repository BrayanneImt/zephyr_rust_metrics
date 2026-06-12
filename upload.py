"""
upload.py — Envoi d'un fichier .wasm vers l'équipement IoT via UART

Protocole :
  1. Envoyer 4 octets = taille du fichier (little-endian uint32)
  2. Envoyer le binaire .wasm octet par octet
"""

import struct
import time
import sys
import os

import serial

# ----------------------------------------------------------------
# Configuration
# ----------------------------------------------------------------
PORT      = '/dev/ttyUSB0'
BAUD      = 115200
WASM_FILE = 'metrics_wasm.wasm'

# Délai après ouverture du port (laisse l'UART ESP32 se stabiliser)
UART_SETTLE_SECS = 0.5

# Taille des chunks pour l'envoi (améliore la progression)
CHUNK_SIZE = 256


def progress_bar(done: int, total: int, width: int = 40) -> str:
    pct  = done / total
    fill = int(width * pct)
    bar  = '=' * fill + '-' * (width - fill)
    return f'[{bar}] {done}/{total} octets ({pct*100:.1f}%)'


def main():
    # Vérifier que le fichier existe
    if not os.path.isfile(WASM_FILE):
        print(f'ERREUR : fichier introuvable : {WASM_FILE}')
        print('Compiler d\'abord avec : bash build_wasm.sh')
        sys.exit(1)

    with open(WASM_FILE, 'rb') as f:
        data = f.read()

    wasm_size = len(data)
    print(f'Fichier       : {WASM_FILE}')
    print(f'Taille        : {wasm_size} octets')
    print(f'Port série    : {PORT} @ {BAUD} baud')
    print()

    # Ouvrir le port série
    try:
        ser = serial.Serial(PORT, BAUD, timeout=5)
    except serial.SerialException as e:
        print(f'ERREUR port série : {e}')
        print('Vérifier que /dev/ttyUSB0 existe et que vous avez les droits.')
        print('  sudo usermod -aG dialout $USER  (puis se reconnecter)')
        sys.exit(1)

    # Délai de stabilisation UART
    print(f'Stabilisation UART ({UART_SETTLE_SECS}s)...')
    time.sleep(UART_SETTLE_SECS)

    # Vider le buffer de réception (restes éventuels de logs Zephyr)
    ser.reset_input_buffer()

    # Envoyer la taille (4 octets, little-endian)
    print('Envoi de la taille...')
    ser.write(struct.pack('<I', wasm_size))
    ser.flush()

    # Délai pour laisser l'équipement lire les 4 octets
    time.sleep(0.1)

    # Envoyer le binaire WASM par chunks
    print('Envoi du binaire WASM...')
    sent = 0
    while sent < wasm_size:
        chunk = data[sent:sent + CHUNK_SIZE]
        ser.write(chunk)
        sent += len(chunk)
        print(f'\r  {progress_bar(sent, wasm_size)}', end='', flush=True)

    ser.flush()
    print()  # Nouvelle ligne après la barre de progression

    print()
    print('UPLOAD DONE')
    print()
    print('L\'équipement devrait afficher :')
    print('  Incoming size = ... bytes')
    print('  Upload complete (... bytes)')
    print('  Module charge OK')
    print('  Instance creee OK')
    print('  Executing WASM...')
    print()
    print('Surveiller avec : west espressif monitor')

    ser.close()


if __name__ == '__main__':
    main()