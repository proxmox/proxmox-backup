include defines.mk

ARCH:=$(shell dpkg-architecture -qDEB_BUILD_ARCH)
GITVERSION:=$(shell git rev-parse HEAD)

SUBDIRS := etc www docs

# Binaries usable by users
USR_BIN := \
	proxmox-backup-client \
	pxar

# Binaries usable by admins
USR_SBIN := proxmox-backup-manager

# Binaries for services:
SERVICE_BIN := \
	proxmox-backup-api \
	proxmox-backup-proxy

ifeq ($(BUILD_MODE), release)
CARGO_BUILD_ARGS += --release
COMPILEDIR := target/release
else
COMPILEDIR := target/debug
endif

COMPILED_BINS := \
	$(addprefix $(COMPILEDIR)/,$(USR_BIN) $(USR_SBIN) $(SERVICE_BIN))

export PROXMOX_PKG_VERSION=${PKGVER}
export PROXMOX_PKG_RELEASE=${PKGREL}
export PROXMOX_PKG_REPOID=${GITVERSION}

export PROXMOX_JSDIR := $(JSDIR)
export PROXMOX_CONFIGDIR := $(SYSCONFDIR)/proxmox-backup

DEB=${PACKAGE}_${PKGVER}-${PKGREL}_${ARCH}.deb
DSC=${PACKAGE}_${PKGVER}-${PKGREL}.dsc

DESTDIR=

all: cargo-build $(SUBDIRS)

.PHONY: $(SUBDIRS)
$(SUBDIRS):
	$(MAKE) -C $@

test:
	#cargo test test_broadcast_future
	#cargo test $(CARGO_BUILD_ARGS)
	cargo test $(tests) $(CARGO_BUILD_ARGS)

doc:
	cargo doc --no-deps $(CARGO_BUILD_ARGS)

# always re-create this dir
# but also copy the local target/ dir as a build-cache
.PHONY: build
build:
	rm -rf build
	cargo build --release
	rsync -a debian Makefile defines.mk Cargo.toml Cargo.lock \
	    src proxmox-protocol zstd-sys $(SUBDIRS) \
	    target build/
	$(foreach i,$(SUBDIRS), \
	    $(MAKE) -C build/$(i) clean ;)

.PHONY: deb
deb: $(DEB)
$(DEB): build
	cd build; dpkg-buildpackage -b -us -uc --no-pre-clean
	lintian $(DEB)

.PHONY: dsc
dsc: $(DSC)
$(DSC): build
	cd build; dpkg-buildpackage -S -us -uc -d -nc
	lintian $(DSC)

distclean: clean

clean:
	$(foreach i,$(SUBDIRS), \
	    $(MAKE) -C $(i) clean ;)
	cargo clean
	rm -rf *.deb *.dsc *.tar.gz *.buildinfo *.changes build
	find . -name '*~' -exec rm {} ';'

.PHONY: dinstall
dinstall: ${DEB}
	dpkg -i ${DEB}

# make sure we build binaries before docs
docs: cargo-build

.PHONY: cargo-build
cargo-build:
	cargo build $(CARGO_BUILD_ARGS)

$(COMPILED_BINS): cargo-build

install: $(COMPILED_BINS)
	install -dm755 $(DESTDIR)$(BINDIR)
	$(foreach i,$(USR_BIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(BINDIR)/ ;)
	install -dm755 $(DESTDIR)$(SBINDIR)
	$(foreach i,$(USR_SBIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(SBINDIR)/ ;)
	install -dm755 $(DESTDIR)$(LIBEXECDIR)/proxmox-backup
	$(foreach i,$(SERVICE_BIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(LIBEXECDIR)/proxmox-backup/ ;)
	$(MAKE) -C www install
	$(MAKE) -C docs install
