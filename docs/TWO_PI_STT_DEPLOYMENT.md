# Speech-to-text server on the 16 GB Pi (Hailo-10H)

Prepared 2026-11-07 for ticket T3 of [the push-to-talk spec](specs/push-to-talk.md). **Status (2026-11-07):** done. The server was built and tested in user space (section 5), then, with Darren's OK (D10), installed as the systemd service (section 6), moved to DKMS (section 7) and checked across a reboot of the 16 GB Pi (section 8). Each of those sections ends with a **Result** paragraph from the run; one command in section 7 differs from the first draft (`--force`) and is fixed in the text below.

## What runs where

| Host | Responsibility |
| --- | --- |
| **16 GB Pi: jarvis-ai** | Gemma on 8080, BGE on 8081, and `jarvis-stt` on **8082**: HailoRT's `Speech2Text` with Whisper-Medium on the Hailo-10H, behind cpp-httplib. LAN only, no authentication, like the other two. |
| **8 GB Pi: Jarvis** | The Rust app records audio and posts a WAV to `http://jarvis-ai:8082/v1/audio/transcriptions` (T4). Nothing here changes for T3. |

Source: `scripts/stt/` in this repository. Why this server and not whisper-server: spec D12 and D13.

## 1. Behaviour

| Endpoint | Behaviour |
| --- | --- |
| `GET /health` and `GET /v1/health` | The same handler. 200 `{"status":"ok"}`. The server starts listening only after the model has loaded, so a 200 means ready. Jarvis probes `/health` at the root (as for llama-server); `/v1/health` sits next to the API. Both answer while a transcription runs. |
| `POST /v1/audio/transcriptions` | Multipart. Required `file`: WAV, RIFF/PCM, 16 kHz, mono, 16-bit (the `fmt ` and `data` chunks are found by walking the chunks). `language` is optional and must be `en`. `model`, `prompt`, `temperature` and any other field are accepted and ignored (Hailo's API has no prompt: D14). Success: 200 `{"text":"..."}`. |

Rig 0.42's client decodes success as `{"text": ...}` and keeps an error body raw (`providers/internal/transcription.rs`, `send_transcription`), so errors use the OpenAI envelope `{"error":{"message":"...","type":"invalid_request_error"}}`. A HailoRT failure returns 500 with `"type":"server_error"` and the HailoRT status number.

Text rules (`stt_logic.cpp`, tested in `stt_logic_test.cpp`):

- The segment texts are joined; a space is added only where two segments would otherwise glue two words together. HailoRT's text starts with a space (`" Forget that"`); it is returned unchanged and the client trims it.
- A segment that starts at or after the audio duration is dropped (Medium invented "you" at 14.88 to 16.88 s on a 15 s clip).
- If what remains is only whitespace, dots, `…` or bracketed markers (`[BLANK_AUDIO]`, `[music]`, `(silence)`), the text is `""` and Jarvis says "didn't catch that" (D6).

Limits (named constants in `stt_logic.hpp` and `main.cpp`):

| Constant | Value | Why |
| --- | --- | --- |
| `MIN_AUDIO_MS` | 300 (4,800 samples) | Matches `MIN_RECORDING`. 0.29 s is rejected, 0.30 s accepted. |
| `MAX_AUDIO_MS` | 15,500 (248,000 samples) | `MAX_RECORDING_MS` 15,000 plus `MAX_AUDIO_MARGIN_MS` 500. 15.5 s accepted, 15.6 s rejected. |
| `MAX_BODY_BYTES` | 561,536 | 15.5 s x 32,000 B/s = 496,000 B of PCM (`MAX_PCM_BYTES`) plus `BODY_HEADROOM_BYTES` 65,536 for the WAV header, multipart boundaries and text fields. Over it: 413 before the body is read. |
| `REQUEST_READ_TIMEOUT` | 10 s | Longest wait for the next bytes of a request. A POST that announces a body and never sends it gets a 400 after 10 s, not a hang. |
| `GENERATE_TIMEOUT` | 30 s | Passed to HailoRT. A 15 s clip takes about 11 s at most; the client waits 20 s. |
| `HTTP_THREADS` | 4 | At least 2, so health answers while a transcription runs. |

One transcription runs at a time (a mutex around `Speech2Text`); a second request waits its turn.

Privacy (D7): audio stays in memory and is never written to disk. The log has one line per request with the status, request bytes, audio seconds and latency, and never any transcript text or audio. HailoRT writes its own `hailort.log` (device and OS lines, no audio or text) into the working directory.

## 2. Settings

Arguments win over environment variables. The model path is not compiled in, so changing between Medium and Small is a setting plus a restart.

| Argument | Environment variable | Default |
| --- | --- | --- |
| `--model PATH` | `JARVIS_STT_HEF` | none, required. `~/hailo-models/Whisper-Medium.hef` (sha256 `64423764f8138995eb51ef64478ead2ddebeffdf3eec50fe68b8abca0f1613de`); `Whisper-Small.hef` is next to it. |
| `--host ADDR` | `JARVIS_STT_HOST` | `0.0.0.0` |
| `--port N` | `JARVIS_STT_PORT` | `8082` |

## 3. Build (16 GB Pi, user space)

The build needs cmake, g++ and the installed HailoRT 5.5.0 (`find_package(HailoRT 5.5.0)` works with `/usr/lib/cmake/HailoRT`). cpp-httplib is not committed: CMake downloads the header from the v0.65.0 tag and checks its SHA-256 (`17d85dcf129d3cc67eefba846608016f43882c9de7fae811a930f9736a10eeaa`); a mismatch fails the configure. It is used without OpenSSL.

From the Mac, copy the source, then build on the Pi. Keep the load low: Gemma is serving.

```bash
# Mac, from the repository root (the 16 GB Pi is reached through the 8 GB Pi)
scp -o ProxyJump=darrenrasp@jarvis scripts/stt/{CMakeLists.txt,main.cpp,stt_logic.cpp,stt_logic.hpp,stt_logic_test.cpp} \
    raspdarren-ai@jarvis-ai:jarvis-stt/src/

# 16 GB Pi
mkdir -p ~/jarvis-stt/src ~/jarvis-stt/build
cd ~/jarvis-stt
nice -n 10 cmake -S src -B build
nice -n 10 cmake --build build -j3
ctest --test-dir build --output-on-failure
```

The unit tests need no Hailo device or network; on any machine `cmake -S scripts/stt -B /tmp/stt-build -DJARVIS_STT_BUILD_SERVER=OFF && cmake --build /tmp/stt-build && ctest --test-dir /tmp/stt-build` runs them.

## 4. Run by hand

```bash
cd ~/jarvis-stt
JARVIS_STT_HEF=$HOME/hailo-models/Whisper-Medium.hef \
  setsid nohup nice -n 5 ./build/jarvis-stt > server.log 2>&1 < /dev/null &
sleep 10; cat server.log     # "model loaded in ~6000 ms", then "listening on 0.0.0.0:8082"
curl -s localhost:8082/health; curl -s localhost:8082/v1/health
```

Stop it with `kill -TERM $(pgrep -x jarvis-stt)`. SIGTERM and SIGINT stop the listener, let a running transcription finish, release the device and exit 0.

Only one process can hold the model at a time. Stop `jarvis-stt` before running the throwaway trial program or anything else that opens the Hailo device.

## 5. Acceptance (from the 8 GB Pi)

`scripts/stt/acceptance.sh` takes a base URL and a clip directory. It needs bash, curl and python3 (all on the 8 GB Pi) and makes its boundary files from the clips in a temporary directory it removes on exit.

```bash
scp scripts/stt/acceptance.sh darrenrasp@jarvis:/tmp/acceptance.sh        # from the Mac
bash /tmp/acceptance.sh http://jarvis-ai:8082/v1 /tmp/stt-spike           # on the 8 GB Pi
```

It checks `/health` and `/v1/health`; every clip (`clip1` to `clip5` and `clip6-long` return 200 with text, `clip2-jieli` returns `""`); the rejections (no body, no file part, non-WAV, 8 kHz, stereo, 0.29 s, 15.6 s, `language=fr`, a 700 KB body, and a request that announces a body and never sends it); the accepted boundaries (0.30 s, 15.5 s, `language=en`); health during a long transcription; and two requests at once. Run the same checks by hand with curl, for example:

```bash
curl -s -F file=@/tmp/stt-spike/clip3.wav http://jarvis-ai:8082/v1/audio/transcriptions   # {"text":" Forget that"}
curl -s -X POST http://jarvis-ai:8082/v1/audio/transcriptions                              # 400, missing file part
curl -s -F file=@/tmp/stt-spike/clip1.wav -F language=fr http://jarvis-ai:8082/v1/audio/transcriptions   # 400
```

After a run, confirm the neighbours are unaffected:

```bash
# 16 GB Pi
hailortcli fw-control identify          # Firmware Version: 5.5.0, HAILO10H
curl -s localhost:8080/health; curl -s localhost:8081/health
```

## 6. Install as a systemd service (run 2026-11-07)

`scripts/stt/jarvis-stt.service` is a template for the 16 GB Pi (`raspdarren-ai`, binary in `~/jarvis-stt/build`, Medium HEF). It binds `0.0.0.0:8082` like the other two services, restarts on failure, waits up to 60 s for `/dev/h1x-0` before loading the model, and sets `NoNewPrivileges` and `UMask=0077`.

Stop the hand-started server first (section 4), then on the 16 GB Pi:

```bash
test ! -e /etc/systemd/system/jarvis-stt.service        # stop if the name is already taken
scp -o ProxyJump=darrenrasp@jarvis scripts/stt/jarvis-stt.service raspdarren-ai@jarvis-ai:jarvis-stt/jarvis-stt.service   # from the Mac
pgrep -x jarvis-stt || true                              # must print nothing
sudo systemd-analyze verify ~/jarvis-stt/jarvis-stt.service
sudo install -m 0644 ~/jarvis-stt/jarvis-stt.service /etc/systemd/system/jarvis-stt.service
sudo systemctl daemon-reload
sudo systemctl enable --now jarvis-stt.service
sleep 12
systemctl status jarvis-stt.service --no-pager
journalctl -u jarvis-stt.service -n 30 --no-pager
curl -s localhost:8082/health; curl -s localhost:8082/v1/health
```

**Result (2026-11-07):** `systemd-analyze verify` printed nothing and exited 0. After `enable --now`, `is-active` was `active`, the log showed `model loaded in 6062 ms` and `listening on 0.0.0.0:8082`, and `/health` and `/v1/health` both returned 200. `clip1` from the 8 GB Pi returned `" Hi Davies, how's the weather today?"` in 2.49 s. The sudo steps were run as scripts that logged to `~/jarvis-stt/s6a.log` and `s6b.log`.

To switch to Whisper-Small: change `JARVIS_STT_HEF` in `/etc/systemd/system/jarvis-stt.service` to `/home/raspdarren-ai/hailo-models/Whisper-Small.hef`, then `sudo systemctl daemon-reload && sudo systemctl restart jarvis-stt`.

## 7. DKMS for `hailo1x_pci` 5.5.0 (run 2026-11-07)

Why: the driver was copied by hand to `/lib/modules/6.18.50+rpt-rpi-2712/updates/hailo1x_pci.ko`, so a kernel update leaves the HAT without a driver (D13). DKMS rebuilds it for each new kernel. `dkms` 3.2.2 and the `linux-headers-rpi-2712` and `linux-headers-rpi-v8` packages are installed already, so DKMS can build for future kernels. **Do not run `apt full-upgrade` or install a new kernel until this section and section 8 are done.**

The recipe follows the drivers repository's `linux/pcie/dkms.conf.in` (`MAKE="make -C ${kernel_source_dir} M=${BUILD_DIRECTORY} HAILO_DRIVER=pcie modules"`, `DEST_MODULE_LOCATION=/updates/dkms`, `AUTOINSTALL=yes`). Its `@PCIE_DRIVER_VERSION@` is filled in by `sed`.

```bash
# 16 GB Pi
export HAILO_VER=5.5.0
export DKMS_SRC=/usr/src/hailo1x_pci-$HAILO_VER
cd ~/hailo-src/hailort-drivers
git describe --tags                        # expect v5.5.0
make clean                                 # remove the old in-tree build output (runs as the normal user)

test ! -e "$DKMS_SRC"                      # stop if it exists
sudo mkdir "$DKMS_SRC"
sudo cp -a Kbuild Makefile LICENSE common linux "$DKMS_SRC"/
sed "s/@PCIE_DRIVER_VERSION@/$HAILO_VER/" linux/pcie/dkms.conf.in | sudo tee "$DKMS_SRC/dkms.conf"

sudo dkms add     -m hailo1x_pci -v "$HAILO_VER"
sudo dkms build   -m hailo1x_pci -v "$HAILO_VER"
# --force is needed: without it dkms refuses ("already installed at version 5.5.0") because the
# hand-copied module of the same version sits in updates/. With it, dkms archives that file in
# /var/lib/dkms/hailo1x_pci/original_module/ and installs updates/dkms/hailo1x_pci.ko.xz.
sudo dkms install -m hailo1x_pci -v "$HAILO_VER" --force
dkms status                                # hailo1x_pci/5.5.0, 6.18.50+rpt-rpi-2712, aarch64: installed (Original modules exist)

# dkms already moved the hand-copied module away, so this rm is not needed; if the file is still there, remove it.
sudo rm -f /lib/modules/$(uname -r)/updates/hailo1x_pci.ko
sudo depmod -a
find /lib/modules/$(uname -r) -name 'hailo1x_pci*'     # only .../updates/dkms/hailo1x_pci.ko.xz
modinfo -n hailo1x_pci                                 # the same dkms path
```

**Result (2026-11-07):** `dkms add` and `dkms build` succeeded (the kernel has no module signing, so the module is unsigned). The first `dkms install` without `--force` failed with "already installed at version 5.5.0"; it changed nothing, and the retry with `--force` installed. After it: `dkms status` showed `hailo1x_pci/5.5.0, 6.18.50+rpt-rpi-2712, aarch64: installed (Original modules exist)`, and `find` showed only `updates/dkms/hailo1x_pci.ko.xz`. `modinfo` gave that path with `version: 5.5.0`, `vermagic: 6.18.50+rpt-rpi-2712 SMP preempt mod_unload modversions aarch64` (equal to `uname -r`) and the same `srcversion` as the module already loaded. The loaded module stayed in memory and `hailortcli fw-control identify` still answered 5.5.0. Logs: `~/jarvis-stt/s7a.log`, `s7b.log`, `s7c.log`.

The running module is untouched until the reboot. The firmware (`/lib/firmware/hailo/hailo10h/`) and the udev rule (`/etc/udev/rules.d/51-hailo-udev.rules`) do not depend on the kernel and stay.

## 8. Reboot test (run 2026-11-07)

Run after sections 6 and 7. A reboot stops Gemma and BGE for about a minute and Gemma takes about 46 s to load; tell Darren before it.

```bash
sudo systemctl reboot
# wait about 2 minutes, then from the 8 GB Pi (ssh -J) or the Mac:
ls -l /dev/h1x-0                                   # present, mode crw-rw-rw-
dkms status                                        # installed for the running kernel
hailortcli fw-control identify                     # Firmware Version: 5.5.0
systemctl is-enabled jarvis-stt jarvis-gemma jarvis-embeddings
systemctl status jarvis-stt --no-pager
curl -s http://jarvis-ai:8082/health; curl -s http://jarvis-ai:8082/v1/health
curl -s http://jarvis-ai:8080/health; curl -s http://jarvis-ai:8081/health
bash /tmp/acceptance.sh http://jarvis-ai:8082/v1 /tmp/stt-spike    # on the 8 GB Pi
```

**Result (2026-11-07):** one `systemctl reboot` at 17:31. The first login worked about 90 s later, with `uptime` showing 1 minute.
- `/dev/h1x-0` was `crw-rw-rw-`; `lsmod` showed `hailo1x_pci`; `modinfo -n` gave the DKMS copy; `dkms status` showed the module installed for 6.18.50+rpt-rpi-2712; `hailortcli fw-control identify` gave Firmware 5.5.0, HAILO10H.
- `jarvis-gemma`, `jarvis-embeddings` and `jarvis-stt` were all enabled and active. From the 8 GB Pi, `/health` on 8080, 8081 and 8082 and `/v1/health` on 8082 all returned 200. Gemma answered 503 "Loading model" for the first minute after boot, as expected.
- `jarvis-stt` loaded its model in 23.9 s after the boot, not 6 s: the HEF was not in the page cache and Gemma was loading at the same time. It started at 17:32:36 and was listening at 17:33:00.
- `acceptance.sh` from the 8 GB Pi passed 25 of 25 (the same cases as section 5; clip latencies within 0.1 s of the earlier run).
- `systemctl --failed` listed one unit: `NetworkManager-wait-online.service` (it timed out after 60 s). It is not part of this change, and the Pi has no persistent journal, so there is no earlier boot to compare with. Its effect is that every service ordered `After=network-online.target` (Gemma, BGE and `jarvis-stt`) starts about a minute after boot.

If `jarvis-stt` is up but `/dev/h1x-0` is missing, the driver did not load: `dmesg | grep -i hailo`, `lsmod | grep hailo`, then `dkms status`.

## 9. Rollback

Stop and remove the service (section 6):

```bash
sudo systemctl disable --now jarvis-stt.service
sudo rm /etc/systemd/system/jarvis-stt.service && sudo systemctl daemon-reload
```

Undo DKMS (section 7) and restore the hand-copied module. `make clean` in section 7 removed the built module from `~/hailo-src/hailort-drivers`, but dkms archived the hand-copied file at `/var/lib/dkms/hailo1x_pci/original_module/6.18.50+rpt-rpi-2712/aarch64/hailo1x_pci.ko`. Copy it out before `dkms remove`, which can delete the archive (the copy below was not run):

```bash
sudo cp /var/lib/dkms/hailo1x_pci/original_module/$(uname -r)/aarch64/hailo1x_pci.ko /tmp/hailo1x_pci.ko.orig
sudo dkms remove -m hailo1x_pci -v 5.5.0 --all
sudo rm -rf /usr/src/hailo1x_pci-5.5.0
ls /lib/modules/$(uname -r)/updates/                      # dkms may already have put the original back
sudo install -m 644 /tmp/hailo1x_pci.ko.orig /lib/modules/$(uname -r)/updates/hailo1x_pci.ko   # if it is missing
sudo depmod -a
```

Alternatively rebuild it from source with `make pcie` in `~/hailo-src/hailort-drivers` (needs the running kernel's headers).

The full rollback of the HailoRT 5.5.0 install itself is the bold **Rollback** line in `notes/HAILO_STT_RESEARCH.md` (about line 549). The server needs no other undo: it writes nothing but `~/jarvis-stt/server.log` and HailoRT's `hailort.log`.

## 10. Troubleshooting

| Symptom | Check |
| --- | --- |
| `cannot open the Hailo device (status ...)` | `ls -l /dev/h1x-0`, `hailortcli fw-control identify`, and that no other process (a trial program, a second `jarvis-stt`) holds the device. |
| `cannot load the model (status ...)` | The HEF path and size (`ls -l`), and that it is the 5.5.0 HEF (sha256 above). |
| `cannot bind 0.0.0.0:8082` | `ss -ltnp \| grep 8082`: an old whisper-server or a second `jarvis-stt`. |
| Health refused for the first 6 s | Normal: the model loads before the server listens. |
| 413 or 400 for a good recording | Compare the sizes with section 1: more than 15.5 s, less than 0.3 s, or not 16 kHz mono 16-bit. |
| 500 `hailo status N` | Look up N in `hailo_status` in `/usr/include/hailo/hailort.h`; check `hailortcli fw-control identify` and `dmesg \| grep -i hailo`. |
