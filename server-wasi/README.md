# Faasta Server (WASI)

This directory contains the server component of Faasta, a high-performance Function-as-a-Service platform built on WebAssembly and WASI.

## Overview

The Faasta server is responsible for:
- Hosting and executing WebAssembly functions
- Managing TLS certificates
- Handling HTTP and HTTPS traffic
- Providing an RPC interface for function deployment
- Authentication via GitHub OAuth

## Self-Hosting on Ubuntu

This guide will help you set up your own Faasta server instance on Ubuntu.

### Prerequisites

- Ubuntu 20.04 LTS or newer
- Root access or sudo privileges
- A domain name pointing to your server (for TLS certificates)
- Open ports:
  - 80 (HTTP for redirects)
  - 443 (HTTPS for function execution)
  - 4433 (QUIC for RPC service)

### Installation

1. **Create a directory for Faasta**

```bash
sudo mkdir -p /opt/faasta
```

2. **Copy the infrastructure files**

Clone the repository or copy the files from the `server-wasi/infra` directory to your server:

```bash
# If cloning the repository
git clone https://github.com/fourlexboehm/faasta.git
cd faasta/server-wasi/infra

# Copy files to the installation directory
sudo cp *.service *.timer *.sh /opt/faasta/
```

3. **Make the scripts executable**

```bash
sudo chmod +x /opt/faasta/*.sh
```

4. **Run the setup script**

```bash
cd /opt/faasta
sudo ./setup-faasta-autoupdate.sh
```

This script will:
- Create a `faasta` user
- Set up required directories
- Install systemd service files
- Configure automatic updates
- Download the latest Faasta server binary

### Configuration

The Faasta server can be configured through environment variables or command-line arguments. The default configuration is set in the `faasta.service` file.

#### Setting up Porkbun API for Automatic TLS Certificates

Faasta can automatically provision TLS certificates for your domain using the Porkbun API:

1. **Create a Porkbun API key**
   - Log into your Porkbun account
   - Go to "API Access" in your account settings
   - Create a new API key

2. **Set up the environment variables**
   Edit the systemd service file to include Porkbun API credentials:

```bash
sudo systemctl stop faasta
sudo nano /etc/systemd/system/faasta.service
```

Add the following environment variables to the `[Service]` section:

```
Environment=PORKBUN_API_KEY=your_api_key_here
Environment=PORKBUN_SECRET_API_KEY=your_secret_api_key_here
```

3. **Configure the domain settings**
   Make sure your service is configured with:
   
```
--base-domain yourdomain.com
--auto-cert true
```

#### Key Configuration Options

| Option | Description | Default |
|--------|-------------|---------|
| `--base-domain` | Base domain for function subdomains | faasta.xyz |
| `--listen-addr` | Address to listen on for HTTPS | 0.0.0.0:443 |
| `--http-listen-addr` | Address to listen on for HTTP redirects | 0.0.0.0:80 |
| `--tls-cert-path` | Path to TLS certificate file | ./certs/cert.pem |
| `--tls-key-path` | Path to TLS private key file | ./certs/key.pem |
| `--certs-dir` | Directory for certificate storage | ./certs |
| `--auto-cert` | Auto-generate TLS certificate | true |
| `--letsencrypt-email` | Email for Let's Encrypt | admin@faasta.xyz |
| `--db-path` | Path to the database directory | ./data/db |
| `--functions-path` | Path to the functions directory | ./functions |

#### Customizing the Service

To customize the service configuration, edit the systemd service file and reload:

```bash
sudo systemctl daemon-reload
sudo systemctl start faasta
```

### Managing the Service

#### Check Service Status

```bash
sudo systemctl status faasta
```

#### View Logs

```bash
# Server output logs
sudo journalctl -u faasta
```

### Directory Structure

After installation, the following directory structure will be created:

- `/opt/faasta/` - Main installation directory
  - `faasta-server` - The server binary
  - `update-faasta.sh` - Script for updating the server
  - `version.txt` - Current version information
- `/var/lib/faasta/` - Data directory
- `/var/log/` - Log files

### Troubleshooting

If you're experiencing issues:

1. Check service status: `sudo systemctl status faasta`
2. View logs: `sudo journalctl -u faasta`
3. Verify file permissions and ownership
4. Ensure required ports are not already in use

### Security Considerations

The Faasta server runs with several security measures:

- Runs as a dedicated `faasta` user with limited privileges
- Uses systemd security features like `ProtectSystem=full` and `PrivateTmp=true`
- Functions are isolated through WebAssembly's sandboxed execution model

For production deployments, consider:

- Setting up a firewall to restrict access to necessary ports
- Using a reverse proxy like Nginx for additional security layers
- Regularly updating the server with the latest security patches

## Advanced Configuration

For advanced configuration options and customization, refer to the source code and comments in the main server files:

- `main.rs` - Main server implementation
- `cert_manager.rs` - TLS certificate management
- `github_auth.rs` - GitHub authentication
- `rpc_service.rs` - RPC service for function deployment
