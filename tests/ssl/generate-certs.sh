#!/usr/bin/env bash
set -euo pipefail

CERTS_DIR="$(cd "$(dirname "$0")" && pwd)/certs"
mkdir -p "$CERTS_DIR"

# Create CA if missing
if [[ ! -f "$CERTS_DIR/ca.key" ]]; then
  openssl genrsa -out "$CERTS_DIR/ca.key" 4096
  openssl req -x509 -new -nodes -key "$CERTS_DIR/ca.key" -sha256 -days 3650 \
    -subj "/CN=Kalatori Test CA" -out "$CERTS_DIR/ca.crt"
fi

gen_cert() {
  local name="$1"
  local cn="$2"
  local key="$CERTS_DIR/${name}.key"
  local csr="$CERTS_DIR/${name}.csr"
  local crt="$CERTS_DIR/${name}.crt"
  local conf="$CERTS_DIR/${name}.ext"

  [[ -f "$key" ]] || openssl genrsa -out "$key" 2048
  openssl req -new -key "$key" -subj "/CN=${cn}" -out "$csr"

  cat > "$conf" <<EOF
basicConstraints=CA:FALSE
keyUsage = digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = @alt_names
[alt_names]
DNS.1 = ${cn}
EOF

  openssl x509 -req -in "$csr" -CA "$CERTS_DIR/ca.crt" -CAkey "$CERTS_DIR/ca.key" -CAcreateserial \
    -out "$crt" -days 3650 -sha256 -extfile "$conf"
}

gen_cert polkadot wss-polkadot
gen_cert statemint wss-statemint

echo "Certificates generated in $CERTS_DIR"

