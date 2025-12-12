# Polyglot-AI Certificate Generation Script for Windows
# Generates TLS certificates for server and client authentication

param(
    [string]$OutputDir = ".\certs",
    [string]$CommonName = "polyglot-ai",
    [int]$ValidDays = 365,
    [switch]$Force
)

$ErrorActionPreference = "Stop"

function Write-Info {
    param([string]$Message)
    Write-Host "[INFO] $Message" -ForegroundColor Cyan
}

function Write-Success {
    param([string]$Message)
    Write-Host "[OK] $Message" -ForegroundColor Green
}

function Write-Warning {
    param([string]$Message)
    Write-Host "[WARN] $Message" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "============================================" -ForegroundColor Cyan
Write-Host "Polyglot-AI Certificate Generator" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
Write-Host ""

# Create output directory
if (-not (Test-Path $OutputDir)) {
    New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
    Write-Info "Created directory: $OutputDir"
}

$OutputDir = Resolve-Path $OutputDir

# Check if certificates already exist
$existingCerts = @(
    "$OutputDir\ca.crt",
    "$OutputDir\ca.key",
    "$OutputDir\server.crt",
    "$OutputDir\server.key"
)

$hasExisting = $existingCerts | Where-Object { Test-Path $_ }
if ($hasExisting -and -not $Force) {
    Write-Warning "Certificates already exist in $OutputDir"
    Write-Warning "Use -Force to overwrite existing certificates"
    exit 0
}

# Check for OpenSSL
$opensslPath = $null
$possiblePaths = @(
    "openssl",
    "C:\Program Files\OpenSSL-Win64\bin\openssl.exe",
    "C:\Program Files\Git\usr\bin\openssl.exe",
    "C:\Program Files (x86)\OpenSSL\bin\openssl.exe"
)

foreach ($path in $possiblePaths) {
    if (Get-Command $path -ErrorAction SilentlyContinue) {
        $opensslPath = $path
        break
    }
}

if (-not $opensslPath) {
    Write-Warning "OpenSSL not found. Attempting to use PowerShell certificate generation..."

    # Use PowerShell's PKI module for certificate generation
    try {
        # Generate CA certificate
        Write-Info "Generating CA certificate..."
        $caParams = @{
            Type = "Custom"
            Subject = "CN=$CommonName CA"
            KeyUsage = "CertSign", "CRLSign"
            KeyAlgorithm = "RSA"
            KeyLength = 4096
            HashAlgorithm = "SHA256"
            NotAfter = (Get-Date).AddDays($ValidDays)
            CertStoreLocation = "Cert:\CurrentUser\My"
            KeyExportPolicy = "Exportable"
        }
        $caCert = New-SelfSignedCertificate @caParams

        # Generate Server certificate
        Write-Info "Generating server certificate..."
        $serverParams = @{
            Type = "Custom"
            Subject = "CN=$CommonName"
            DnsName = $CommonName, "localhost"
            KeyUsage = "DigitalSignature", "KeyEncipherment"
            KeyAlgorithm = "RSA"
            KeyLength = 2048
            HashAlgorithm = "SHA256"
            NotAfter = (Get-Date).AddDays($ValidDays)
            CertStoreLocation = "Cert:\CurrentUser\My"
            KeyExportPolicy = "Exportable"
            Signer = $caCert
            TextExtension = @("2.5.29.37={text}1.3.6.1.5.5.7.3.1")
        }
        $serverCert = New-SelfSignedCertificate @serverParams

        # Export certificates
        Write-Info "Exporting certificates..."

        # Export CA certificate
        Export-Certificate -Cert $caCert -FilePath "$OutputDir\ca.cer" -Type CERT | Out-Null
        $caBytes = $caCert.Export("Cert")
        $caPem = "-----BEGIN CERTIFICATE-----`n"
        $caPem += [Convert]::ToBase64String($caBytes, "InsertLineBreaks")
        $caPem += "`n-----END CERTIFICATE-----"
        Set-Content -Path "$OutputDir\ca.crt" -Value $caPem -NoNewline

        # Export CA private key
        $caKeyBytes = $caCert.PrivateKey.ExportRSAPrivateKey()
        $caKeyPem = "-----BEGIN RSA PRIVATE KEY-----`n"
        $caKeyPem += [Convert]::ToBase64String($caKeyBytes, "InsertLineBreaks")
        $caKeyPem += "`n-----END RSA PRIVATE KEY-----"
        Set-Content -Path "$OutputDir\ca.key" -Value $caKeyPem -NoNewline

        # Export server certificate
        $serverBytes = $serverCert.Export("Cert")
        $serverPem = "-----BEGIN CERTIFICATE-----`n"
        $serverPem += [Convert]::ToBase64String($serverBytes, "InsertLineBreaks")
        $serverPem += "`n-----END CERTIFICATE-----"
        Set-Content -Path "$OutputDir\server.crt" -Value $serverPem -NoNewline

        # Export server private key
        $serverKeyBytes = $serverCert.PrivateKey.ExportRSAPrivateKey()
        $serverKeyPem = "-----BEGIN RSA PRIVATE KEY-----`n"
        $serverKeyPem += [Convert]::ToBase64String($serverKeyBytes, "InsertLineBreaks")
        $serverKeyPem += "`n-----END RSA PRIVATE KEY-----"
        Set-Content -Path "$OutputDir\server.key" -Value $serverKeyPem -NoNewline

        # Clean up certificates from store
        Remove-Item "Cert:\CurrentUser\My\$($caCert.Thumbprint)" -Force
        Remove-Item "Cert:\CurrentUser\My\$($serverCert.Thumbprint)" -Force

        Write-Success "Certificates generated successfully using PowerShell"
    }
    catch {
        Write-Error "Failed to generate certificates: $_"
        Write-Host ""
        Write-Host "Please install OpenSSL and try again, or use the Rust binary:" -ForegroundColor Yellow
        Write-Host "  polyglot-server generate-certs -o $OutputDir" -ForegroundColor White
        exit 1
    }
}
else {
    Write-Info "Using OpenSSL: $opensslPath"

    # Create OpenSSL config file
    $opensslConfig = @"
[req]
default_bits = 2048
prompt = no
default_md = sha256
distinguished_name = dn
x509_extensions = v3_ca

[dn]
CN = $CommonName

[v3_ca]
basicConstraints = critical, CA:TRUE
keyUsage = critical, keyCertSign, cRLSign
subjectKeyIdentifier = hash

[server_ext]
basicConstraints = CA:FALSE
keyUsage = digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = @alt_names
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:always

[alt_names]
DNS.1 = $CommonName
DNS.2 = localhost
IP.1 = 127.0.0.1

[client_ext]
basicConstraints = CA:FALSE
keyUsage = digitalSignature
extendedKeyUsage = clientAuth
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:always
"@

    $configPath = "$OutputDir\openssl.cnf"
    Set-Content -Path $configPath -Value $opensslConfig

    try {
        # Generate CA key and certificate
        Write-Info "Generating CA key..."
        & $opensslPath genrsa -out "$OutputDir\ca.key" 4096 2>$null

        Write-Info "Generating CA certificate..."
        & $opensslPath req -new -x509 -days $ValidDays -key "$OutputDir\ca.key" -out "$OutputDir\ca.crt" -config $configPath -extensions v3_ca

        # Generate server key and certificate
        Write-Info "Generating server key..."
        & $opensslPath genrsa -out "$OutputDir\server.key" 2048 2>$null

        Write-Info "Generating server CSR..."
        & $opensslPath req -new -key "$OutputDir\server.key" -out "$OutputDir\server.csr" -config $configPath

        Write-Info "Signing server certificate..."
        & $opensslPath x509 -req -days $ValidDays -in "$OutputDir\server.csr" -CA "$OutputDir\ca.crt" -CAkey "$OutputDir\ca.key" -CAcreateserial -out "$OutputDir\server.crt" -extfile $configPath -extensions server_ext

        # Clean up
        Remove-Item "$OutputDir\server.csr" -ErrorAction SilentlyContinue
        Remove-Item "$OutputDir\openssl.cnf" -ErrorAction SilentlyContinue
        Remove-Item "$OutputDir\ca.srl" -ErrorAction SilentlyContinue

        Write-Success "Certificates generated successfully using OpenSSL"
    }
    catch {
        Write-Error "OpenSSL command failed: $_"
        exit 1
    }
}

# Display results
Write-Host ""
Write-Host "============================================" -ForegroundColor Green
Write-Host "Certificates Generated Successfully" -ForegroundColor Green
Write-Host "============================================" -ForegroundColor Green
Write-Host ""
Write-Host "Output directory: $OutputDir" -ForegroundColor White
Write-Host ""
Write-Host "Files created:" -ForegroundColor Cyan
Write-Host "  ca.crt     - CA certificate (distribute to clients)" -ForegroundColor White
Write-Host "  ca.key     - CA private key (keep secure!)" -ForegroundColor White
Write-Host "  server.crt - Server certificate" -ForegroundColor White
Write-Host "  server.key - Server private key" -ForegroundColor White
Write-Host ""
Write-Host "Next steps:" -ForegroundColor Yellow
Write-Host "  1. Copy ca.crt to all client machines" -ForegroundColor White
Write-Host "  2. Generate client certificates using:" -ForegroundColor White
Write-Host "     polyglot generate-certs --ca-cert ca.crt --ca-key ca.key" -ForegroundColor Gray
Write-Host "  3. Configure server.toml with certificate paths" -ForegroundColor White
Write-Host ""
