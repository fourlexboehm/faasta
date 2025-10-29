#!/bin/bash
set -e

echo "Setting up Faasta auto-updater on Ubuntu..."
echo "Using repository: fourlexboehm/faasta"

# Create the faasta user
if ! id -u faasta &>/dev/null; then
  echo "Creating faasta user..."
  useradd -r -s /bin/false faasta
fi

# Create directories
echo "Creating directories..."
mkdir -p /opt/faasta /var/lib/faasta
touch /var/log/faasta.log /var/log/faasta.error.log /var/log/faasta-updater.log

# Set permissions
echo "Setting permissions..."
chown -R faasta:faasta /opt/faasta /var/lib/faasta /var/log/faasta*.log

# Copy files
echo "Installing scripts and service files..."
cp update-faasta.sh /opt/faasta/
chmod +x /opt/faasta/update-faasta.sh

# Install service files
cp faasta.service faasta-updater.service faasta-updater.timer /etc/systemd/system/

# Reload systemd
echo "Reloading systemd configuration..."
systemctl daemon-reload

# Enable and start timer
echo "Enabling and starting services..."
systemctl enable faasta-updater.timer
systemctl start faasta-updater.timer

# Enable faasta service so it starts on boot
systemctl enable faasta.service

# Run the update script once to download the latest version
echo "Downloading the latest Faasta release..."
/opt/faasta/update-faasta.sh

echo "Setup complete! Faasta will automatically update every hour."
echo "You can check the logs at:"
echo "  - /var/log/faasta.log (server output)"
echo "  - /var/log/faasta.error.log (server errors)"
echo "  - /var/log/faasta-updater.log (update process)"
