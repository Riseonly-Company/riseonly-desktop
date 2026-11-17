#!/bin/bash
set -euo pipefail

repo="${1:?usage: install-runner.sh <owner/repo> linux|windows}"
kind="${2:?usage: install-runner.sh <owner/repo> linux|windows}"
case "$kind" in linux | windows) ;; *) echo "kind is linux or windows" >&2; exit 2 ;; esac
version=2.338.2
base=/srv/ci/desktop-runners/$kind

id ghrunner >/dev/null 2>&1 || useradd --system --create-home --shell /usr/sbin/nologin ghrunner
usermod -aG docker ghrunner
mkdir -p "$base/home" /srv/ci/desktop
chown -R ghrunner:ghrunner "$base" /srv/ci/desktop
cd "$base"
if [ ! -x ./config.sh ]; then
    curl -fsSL -o runner.tgz "https://github.com/actions/runner/releases/download/v$version/actions-runner-linux-x64-$version.tar.gz"
    tar -xzf runner.tgz && rm runner.tgz
    chown -R ghrunner:ghrunner "$base"
fi
read -rsp "Registration token: " token
echo
sudo -u ghrunner HOME="$base/home" ./config.sh --unattended --url "https://github.com/$repo" --token "$token" \
    --name "riseonly-desktop-$kind" --labels "riseonly-desktop-$kind" --work _work --replace
unset token

cat > /etc/systemd/system/riseonly-desktop-runner@.service <<'UNIT'
[Unit]
Description=GitHub Actions runner %i (riseonly-desktop)
After=network-online.target docker.service
[Service]
User=ghrunner
WorkingDirectory=/srv/ci/desktop-runners/%i
Environment=HOME=/srv/ci/desktop-runners/%i/home
ExecStart=/srv/ci/desktop-runners/%i/run.sh
Restart=always
RestartSec=5
[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable --now "riseonly-desktop-runner@$kind"
systemctl --no-pager status "riseonly-desktop-runner@$kind" | head -3
