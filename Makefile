.PHONY: vpk ftp run

RUSTFLAGS ?= -C target-feature=-neon -A internal_features
CARGO_VITA ?= cargo +nightly vita
VPK_PATH := target/armv7-sony-vita-newlibeabihf/release/vita_deck.vpk
VITA_IP ?= 192.168.1.100 # Cambia esta IP por la de tu PS Vita
FTP_PORT ?= 1337

# macOS's host `ar` refuses to archive ARM ELF objects (ranlib: "not a mach-o
# file"), silently producing an empty static lib. Force the vitasdk cross
# archiver so `ring`'s compiled C/asm objects actually get linked in.
export AR_armv7_sony_vita_newlibeabihf := arm-vita-eabi-ar

vpk:
	RUSTFLAGS="$(RUSTFLAGS)" $(CARGO_VITA) build vpk --release

ftp: vpk
	@echo "Subiendo $(VPK_PATH) a la PS Vita por FTP (IP: $(VITA_IP):$(FTP_PORT))..."
	curl -T $(VPK_PATH) ftp://$(VITA_IP):$(FTP_PORT)/ux0:/data/vita_deck.vpk
	@echo "Subida completada. ¡Instálalo en tu PS Vita usando VitaShell!"

run: ftp
	@echo "Para auto-iniciar necesitas tener el plugin netcat o similar configurado, por ahora, inícialo manualmente."
