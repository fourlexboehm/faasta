#!/bin/bash
set -e

# Configuration
GITHUB_REPO="fourlexboehm/faasta"
INSTALL_DIR="/opt/faasta"
LOG_FILE="/var/log/faasta-updater.log"
DATA_DIR="/var/lib/faasta"

# Function to log messages
log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $1" | tee -a "$LOG_FILE"
}

# Create directories if they don't exist
mkdir -p "$INSTALL_DIR" "$DATA_DIR"
touch "$LOG_FILE"

log "Starting Faasta update process"

# Get all releases (not just the latest) since we're using build-X format
log "Fetching releases from GitHub"
RELEASES=$(curl -s "https://api.github.com/repos/$GITHUB_REPO/releases")

# Check if the API returned valid data
if [ -z "$RELEASES" ] || [ "$(echo "$RELEASES" | grep -c "tag_name")" -eq 0 ]; then
  log "Error: Could not fetch releases from GitHub. Response: $RELEASES"
  exit 1
fi

# Extract the latest build number
LATEST_BUILD=$(echo "$RELEASES" | grep -Po '"tag_name": "build-\K[0-9]+(?=")' | sort -rn | head -1)

if [ -z "$LATEST_BUILD" ]; then
  log "Error: Could not find any builds with tag_name format 'build-X'"
  exit 1
fi

log "Latest build found: build-$LATEST_BUILD"

# Check if we already have this version
if [ -f "$INSTALL_DIR/version.txt" ] && [ "$(cat "$INSTALL_DIR/version.txt")" = "build-$LATEST_BUILD" ]; then
  log "Already running the latest version: build-$LATEST_BUILD"
else
  log "New version found: build-$LATEST_BUILD"
  
  # Use the exact URL format from the example
  DOWNLOAD_URL="https://github.com/$GITHUB_REPO/releases/download/build-$LATEST_BUILD/faasta-server-linux-x86_64"
  
  log "Downloading from $DOWNLOAD_URL"
  
  # Create a temporary directory for downloads
  TMP_DIR=$(mktemp -d)
  TMP_FILE="$TMP_DIR/faasta-server.download"
  
  # Download the file
  if ! curl -L "$DOWNLOAD_URL" -o "$TMP_FILE"; then
    log "Error: Download failed from $DOWNLOAD_URL"
    rm -rf "$TMP_DIR"
    exit 1
  fi
  
  # Make it executable
  chmod +x "$TMP_FILE"
  
  # Stop the existing service if it's running
  if systemctl is-active --quiet faasta; then
    log "Stopping Faasta service"
    systemctl stop faasta
  fi
  
  # Replace the old binary
  mv "$TMP_FILE" "$INSTALL_DIR/faasta-server"
  
  # Clean up
  rm -rf "$TMP_DIR"
  
  # Save the current version
  echo "build-$LATEST_BUILD" > "$INSTALL_DIR/version.txt"
  
  # Start the service
  log "Starting Faasta service"
  systemctl start faasta
  
  log "Update completed successfully to version build-$LATEST_BUILD"
fi
