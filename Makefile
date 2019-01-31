include defines.mk

ARCH:=$(shell dpkg-architecture -qDEB_BUILD_ARCH)
GITVERSION:=$(shell git rev-parse HEAD)

# Binaries usable by users
USR_BIN := \
	proxmox-backup-client \
	catar

# Binaries usable by admins
USR_SBIN := proxmox-backup-manager

# Binaries for services:
SERVICE_BIN := \
	proxmox-backup-api \
	proxmox-backup-proxy

COMPILEDIR := target/release
COMPILED_BINS := \
	$(addprefix $(COMPILEDIR)/,$(USR_BIN) $(USR_SBIN) $(SERVICE_BIN))

export PROXMOX_PKG_VERSION=${PKGVER}
export PROXMOX_PKG_RELEASE=${PKGREL}
export PROXMOX_PKG_REPOID=${GITVERSION}

export PROXMOX_JSDIR := $(JSDIR)

DEB=${PACKAGE}_${PKGVER}-${PKGREL}_${ARCH}.deb

DESTDIR=

all:
	cargo build

test:
	cargo test

.PHONY: deb
deb ${DEB}:
	rm -rf build
	# build here to cache results
	cargo build --release
	$(MAKE) -C www
	rsync -a debian Cargo.toml src www etc target build
	cd build; dpkg-buildpackage -b -us -uc


distclean: clean

clean:
	$(MAKE) -C www clean
	cargo clean
	rm -rf *.deb *.buildinfo *.changes build
	find . -name '*~' -exec rm {} ';'

.PHONY: dinstall
dinstall: ${DEB}
	dpkg -i ${DEB}

.PHONY: build-release
build-release:
	cargo build --release

$(COMPILED_BINS): build-release

install: $(COMPILED_BINS)
	install -dm755 $(DESTDIR)$(BINDIR)
	$(foreach i,$(USR_BIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(BINDIR)/ ;)
	install -dm755 $(DESTDIR)$(SBINDIR)
	$(foreach i,$(USR_SBIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(SBINDIR)/ ;)
	install -dm755 $(DESTDIR)$(LIBDIR)/proxmox-backup
	$(foreach i,$(SERVICE_BIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(LIBDIR)/proxmox-backup/ ;)
	$(MAKE) -C www install
