# Panduan Deployment Linux & Operasional Produksi (Apex-Trade)

Panduan teknis ini menjelaskan prosedur instalasi, pengerasan keamanan server, manajemen daemon systemd, rotasi log, dan pemulihan darurat untuk binary `apex` pada distribusi Linux (Ubuntu 22.04/24.04 LTS atau Debian 12).

---

## 1. Pembuatan Pengguna & Struktur Direktori

Jalankan perintah berikut sebagai `root` untuk membuat pengguna sistem tanpa hak akses sudo:

```bash
# Buat service user khusus
sudo useradd --system --no-create-home --user-group --shell /usr/sbin/nologin apex

# Buat struktur direktori aplikasi
sudo mkdir -p /opt/apex/bin
sudo mkdir -p /etc/apex/config
sudo mkdir -p /var/lib/apex/state
sudo mkdir -p /var/lib/apex/audit
sudo mkdir -p /var/backups/apex-state

# Atur kepemilikan dan hak akses
sudo chown -R apex:apex /opt/apex /var/lib/apex
sudo chmod 0750 /var/lib/apex/state /var/lib/apex/audit
sudo chown -R root:apex /etc/apex
sudo chmod 0750 /etc/apex
```

Salin binary hasil kompilasi:
```bash
sudo cp target/release/apex /opt/apex/bin/apex
sudo chmod 0755 /opt/apex/bin/apex
```

Salin file konfigurasi awal:
```bash
sudo cp config/*.toml /etc/apex/config/
sudo cp config/*.json /etc/apex/config/
sudo chown -R root:apex /etc/apex/config
sudo chmod 0640 /etc/apex/config/*
```

---

## 2. Kredensial Environment (`/etc/apex/apex.env`)

Buat berkas kredensial terisolasi:
```bash
sudo touch /etc/apex/apex.env
sudo chmod 0600 /etc/apex/apex.env
sudo chown apex:apex /etc/apex/apex.env
```

Isi `/etc/apex/apex.env`:
```ini
BINANCE_API_KEY=kunci_api_binance_anda_di_sini
BINANCE_API_SECRET=kunci_rahasia_binance_anda_di_sini
APEX_NOTIFY_URL=https://webhook.site/endpoint-notifikasi-anda
# APEX_LIVE_CONFIRM=I_UNDERSTAND_LIVE_ORDERS # HANYA jika masuk tahap live (G3)
```

---

## 3. Sinkronisasi Waktu Presisi Tinggi (Chrony NTP)

Keabsahan request signed Binance mensyaratkan selisih jam $\le 500\text{ ms}$ (`recvWindow = 5000`).

```bash
# Instal chrony
sudo apt-get update && sudo apt-get install -y chrony

# Verifikasi status sinkronisasi
chronyc tracking
chronyc sources -v
```

Pastikan nilai `System time` offset di bawah $10\text{ ms}$.

---

## 4. Konfigurasi Firewall Egress (UFW / iptables)

Batasi koneksi keluar server hanya ke domain Binance dan port NTP/DNS:

```bash
# Izinkan loopback & DNS/NTP outbound
sudo ufw default deny outgoing
sudo ufw default deny incoming
sudo ufw allow out 53/udp
sudo ufw allow out 53/tcp
sudo ufw allow out 123/udp

# Izinkan SSH masuk (khusus IP manajemen Anda)
sudo ufw allow in proto tcp to any port 22

# Izinkan HTTPS outbound ke internet (REST & WebSocket Binance)
sudo ufw allow out 443/tcp

sudo ufw enable
```

---

## 5. Rotasi Log (`journald`)

Tambahkan konfigurasi batas ukuran log pada `/etc/systemd/journald.conf.d/apex.conf`:
```ini
[Journal]
SystemMaxUse=1G
SystemMaxFileSize=100M
MaxRetentionSec=14day
```
Terapkan dengan:
```bash
sudo systemctl restart systemd-journald
```

Membaca log operasional live:
```bash
journalctl -u apex -f -n 200
```

---

## 6. Otomasi Pencadangan State Direktori (Cron)

Pasang crontab untuk pengguna `root`:
```bash
sudo crontab -e
```
Tambahkan entri pencadangan setiap jam:
```cron
15 * * * * tar -czf /var/backups/apex-state/state-$(date +\%Y\%m\%d-\%H).tar.gz -C /var/lib/apex state
30 2 * * * find /var/backups/apex-state -name "*.tar.gz" -mtime +14 -delete
```

---

## 7. Prosedur Darurat & Pemulihan

### 7.1 Mengaktifkan Kill Switch Seketika
Untuk langsung memblokir order entry baru tanpa mematikan proses monitoring:
```bash
sudo touch /var/lib/apex/state/KILL
```
Untuk melepas kill switch setelah kondisi normal:
```bash
sudo rm -f /var/lib/apex/state/KILL
```

### 7.2 Pemulihan dari State Cadangan (`.bak`)
Bila listrik padam mendadak saat write dan berkas `oms_state.json` utama korup, `StateStore` secara otomatis memuat `oms_state.json.bak`. Jika perlu pemulihan manual:
```bash
sudo systemctl stop apex
cd /var/lib/apex/state/<mode>/
ls -la oms_state.json*
cp oms_state.json.bak oms_state.json
sudo systemctl start apex
```
Periksa status pemulihan di log:
```bash
journalctl -u apex -n 50 | grep -i "recovered_from_bak"
```
