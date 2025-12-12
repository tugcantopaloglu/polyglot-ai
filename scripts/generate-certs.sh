#!/bin/bash
# Polyglot-AI Certificate Generation Script
# Generates CA, server, and client certificates for mTLS

set -e

# Default values
OUTPUT_DIR="${1:-./certs}"
CA_CN="${2:-Polyglot-AI CA}"
SERVER_CN="${3:-polyglot-ai}"
CLIENT_CN="${4:-polyglot-client}"
DAYS="${5:-365}"

echo "==================================="
echo "Polyglot-AI Certificate Generator"
echo "==================================="
echo
echo "Output directory: $OUTPUT_DIR"
echo "CA Common Name: $CA_CN"
echo "Server Common Name: $SERVER_CN"
echo "Client Common Name: $CLIENT_CN"
echo "Validity: $DAYS days"
echo

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Check for openssl
if ! command -v openssl &> /dev/null; then
    echo "Error: openssl is required but not installed."
    exit 1
fi

echo "Generating CA certificate..."
# Generate CA private key
openssl genrsa -out "$OUTPUT_DIR/ca.key" 4096

# Generate CA certificate
openssl req -new -x509 -days "$DAYS" -key "$OUTPUT_DIR/ca.key" \
    -out "$OUTPUT_DIR/ca.crt" \
    -subj "/CN=$CA_CN/O=Polyglot-AI/C=US"

echo "CA certificate generated."
echo

echo "Generating server certificate..."
# Generate server private key
openssl genrsa -out "$OUTPUT_DIR/server.key" 2048

# Generate server CSR
openssl req -new -key "$OUTPUT_DIR/server.key" \
    -out "$OUTPUT_DIR/server.csr" \
    -subj "/CN=$SERVER_CN/O=Polyglot-AI/C=US"

# Create server extensions file
cat > "$OUTPUT_DIR/server_ext.cnf" << EOF
authorityKeyIdentifier=keyid,issuer
basicConstraints=CA:FALSE
keyUsage = digitalSignature, nonRepudiation, keyEncipherment, dataEncipherment
subjectAltName = @alt_names

[alt_names]
DNS.1 = $SERVER_CN
DNS.2 = localhost
IP.1 = 127.0.0.1
IP.2 = ::1
EOF

# Sign server certificate with CA
openssl x509 -req -in "$OUTPUT_DIR/server.csr" \
    -CA "$OUTPUT_DIR/ca.crt" -CAkey "$OUTPUT_DIR/ca.key" \
    -CAcreateserial -out "$OUTPUT_DIR/server.crt" \
    -days "$DAYS" -extfile "$OUTPUT_DIR/server_ext.cnf"

echo "Server certificate generated."
echo

echo "Generating client certificate..."
# Generate client private key
openssl genrsa -out "$OUTPUT_DIR/client.key" 2048

# Generate client CSR
openssl req -new -key "$OUTPUT_DIR/client.key" \
    -out "$OUTPUT_DIR/client.csr" \
    -subj "/CN=$CLIENT_CN/O=Polyglot-AI/C=US"

# Create client extensions file
cat > "$OUTPUT_DIR/client_ext.cnf" << EOF
authorityKeyIdentifier=keyid,issuer
basicConstraints=CA:FALSE
keyUsage = digitalSignature, nonRepudiation, keyEncipherment, dataEncipherment
extendedKeyUsage = clientAuth
EOF

# Sign client certificate with CA
openssl x509 -req -in "$OUTPUT_DIR/client.csr" \
    -CA "$OUTPUT_DIR/ca.crt" -CAkey "$OUTPUT_DIR/ca.key" \
    -CAcreateserial -out "$OUTPUT_DIR/client.crt" \
    -days "$DAYS" -extfile "$OUTPUT_DIR/client_ext.cnf"

echo "Client certificate generated."
echo

# Clean up CSR and extension files
rm -f "$OUTPUT_DIR"/*.csr "$OUTPUT_DIR"/*.cnf "$OUTPUT_DIR"/*.srl

# Set permissions
chmod 600 "$OUTPUT_DIR"/*.key
chmod 644 "$OUTPUT_DIR"/*.crt

echo "==================================="
echo "Certificate Generation Complete!"
echo "==================================="
echo
echo "Generated files in $OUTPUT_DIR:"
echo "  ca.crt      - CA certificate (share with clients)"
echo "  ca.key      - CA private key (keep secure!)"
echo "  server.crt  - Server certificate"
echo "  server.key  - Server private key"
echo "  client.crt  - Client certificate"
echo "  client.key  - Client private key"
echo
echo "For additional clients, run:"
echo "  polyglot-server generate-certs -o ./certs --cn <client-name>"
echo
echo "Certificate fingerprint (for verification):"
openssl x509 -in "$OUTPUT_DIR/ca.crt" -noout -fingerprint -sha256
