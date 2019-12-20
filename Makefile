include /usr/share/dpkg/default.mk
include defines.mk

PACKAGE := $(DEB_SOURCE)
ARCH := $(DEB_BUILD_ARCH)

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

ifeq ($(valgrind), yes)
CARGO_BUILD_ARGS += --features valgrind
endif

CARGO ?= cargo

COMPILED_BINS := \
	$(addprefix $(COMPILEDIR)/,$(USR_BIN) $(USR_SBIN) $(SERVICE_BIN))

DEBS= ${PACKAGE}-server_${DEB_VERSION}_${ARCH}.deb ${PACKAGE}-client_${DEB_VERSION}_${ARCH}.deb

DOC_DEB=${PACKAGE}-docs_${DEB_VERSION}_all.deb

DSC = ${PACKAGE}_${DEB_VERSION}.dsc

DESTDIR=

all: cargo-build $(SUBDIRS)

.PHONY: $(SUBDIRS)
$(SUBDIRS):
	$(MAKE) -C $@

test:
	#cargo test test_broadcast_future
	#cargo test $(CARGO_BUILD_ARGS)
	$(CARGO) test $(tests) $(CARGO_BUILD_ARGS)

doc:
	$(CARGO) doc --no-deps $(CARGO_BUILD_ARGS)

# always re-create this dir
.PHONY: build
build:
	rm -rf build
	rsync -a debian Makefile defines.mk Cargo.toml \
	    src $(SUBDIRS) \
	    tests build/
	$(foreach i,$(SUBDIRS), \
	    $(MAKE) -C build/$(i) clean ;)

.PHONY: proxmox-backup-docs
proxmox-backup-docs: $(DOC_DEB)
$(DOC_DEB): build
	cd build; dpkg-buildpackage -b -us -uc --no-pre-clean
	lintian $(DOC_DEB)

# copy the local target/ dir as a build-cache
.PHONY: deb
deb: $(DEBS)
$(DEBS): build
	cd build; dpkg-buildpackage -b -us -uc --no-pre-clean --build-profiles=nodoc
	lintian $(DEBS)

.PHONY: dsc
dsc: $(DSC)
$(DSC): build
	cd build; dpkg-buildpackage -S -us -uc -d -nc
	lintian $(DSC)

distclean: clean

clean:
	$(foreach i,$(SUBDIRS), \
	    $(MAKE) -C $(i) clean ;)
	$(CARGO) clean
	rm -rf *.deb *.dsc *.tar.gz *.buildinfo *.changes build
	find . -name '*~' -exec rm {} ';'

.PHONY: dinstall
dinstall: ${DEBS}
	dpkg -i ${DEBS}

# make sure we build binaries before docs
docs: cargo-build

.PHONY: cargo-build
cargo-build:
	$(CARGO) build $(CARGO_BUILD_ARGS)

$(COMPILED_BINS): cargo-build

.PHONY: lint
lint:
	cargo clippy -- -A clippy::all -D clippy::correctness

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

.PHONY: upload
upload: ${DEBS}
	# check if working directory is clean
	git diff --exit-code --stat && git diff --exit-code --stat --staged
	tar cf - ${DEBS} | ssh -X repoman@repo.proxmox.com upload --product pbs --dist buster
