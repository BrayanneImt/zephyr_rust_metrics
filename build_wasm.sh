set -e

echo "=== Build : rust_http -> WASM (wasm32-unknown-unknown) ==="

if ! rustup target list --installed | grep -q "wasm32-unknown-unknown"; then
    echo "[setup] Ajout cible wasm32-unknown-unknown..."
    rustup target add wasm32-unknown-unknown
fi

echo "[build] cargo build --release..."
cargo build --target wasm32-unknown-unknown --release

OUTPUT=target/wasm32-unknown-unknown/release/http_wasm.wasm
DEST=http_rust.wasm

SIZE_BEFORE=$(stat -c%s "$OUTPUT")
echo "[build] Module brut : ${SIZE_BEFORE} octets ($(( SIZE_BEFORE / 1024 )) KB)"

# wasm-opt -Oz : optimise la taille ET supprime les reference-types
# (pas de --no-dce ni --export qui ne sont pas supportés par wasm-opt 116)
if command -v wasm-opt &>/dev/null; then
    echo "[opt] wasm-opt -Oz --enable-bulk-memory..."
    wasm-opt -Oz --enable-bulk-memory "$OUTPUT" -o "$DEST"
    SIZE_AFTER=$(stat -c%s "$DEST")
    GAIN=$(( (SIZE_BEFORE - SIZE_AFTER) * 100 / SIZE_BEFORE ))
    echo "[opt] ${SIZE_BEFORE} → ${SIZE_AFTER} octets (-${GAIN}%)"
else
    echo "[opt] wasm-opt absent — copie directe"
    cp "$OUTPUT" "$DEST"
    SIZE_AFTER=$SIZE_BEFORE
fi

echo "[OK] $DEST — ${SIZE_AFTER} octets ($(( SIZE_AFTER / 1024 )) KB)"

MAX=$(( 40 * 1024 ))
if [ "$SIZE_AFTER" -gt "$MAX" ]; then
    echo "ATTENTION : module > ${MAX} octets (WASM_MAX_SIZE)"
fi

echo ""
echo "Déploiement : python3 upload.py"