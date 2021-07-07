include /usr/share/dpkg/default.mk
include defines.mk

PACKAGE := proxmox-backup
ARCH := $(DEB_BUILD_ARCH)

SUBDIRS := etc www docs

# Binaries usable by users
USR_BIN := \
	proxmox-backup-client 	\
	proxmox-file-restore	\
	pxar			\
	proxmox-tape		\
	pmtx			\
	pmt

# Binaries usable by admins
USR_SBIN := \
	proxmox-backup-manager

# Binaries for services:
SERVICE_BIN := \
	proxmox-backup-api \
	proxmox-backup-banner \
	proxmox-backup-proxy \
	proxmox-daily-update

# Single file restore daemon
RESTORE_BIN := \
	proxmox-restore-daemon

SUBCRATES := \
	pbs-api-types \
	pbs-buildcfg \
	pbs-datastore \
	pbs-runtime \
	pbs-systemd \
	pbs-tools

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
	$(addprefix $(COMPILEDIR)/,$(USR_BIN) $(USR_SBIN) $(SERVICE_BIN) $(RESTORE_BIN))

export DEB_VERSION DEB_VERSION_UPSTREAM

SERVER_DEB=${PACKAGE}-server_${DEB_VERSION}_${ARCH}.deb
SERVER_DBG_DEB=${PACKAGE}-server-dbgsym_${DEB_VERSION}_${ARCH}.deb
CLIENT_DEB=${PACKAGE}-client_${DEB_VERSION}_${ARCH}.deb
CLIENT_DBG_DEB=${PACKAGE}-client-dbgsym_${DEB_VERSION}_${ARCH}.deb
RESTORE_DEB=proxmox-backup-file-restore_${DEB_VERSION}_${ARCH}.deb
RESTORE_DBG_DEB=proxmox-backup-file-restore-dbgsym_${DEB_VERSION}_${ARCH}.deb
DOC_DEB=${PACKAGE}-docs_${DEB_VERSION}_all.deb

DEBS=${SERVER_DEB} ${SERVER_DBG_DEB} ${CLIENT_DEB} ${CLIENT_DBG_DEB} \
     ${RESTORE_DEB} ${RESTORE_DBG_DEB}

DSC = rust-${PACKAGE}_${DEB_VERSION}.dsc

DESTDIR=

tests ?= --workspace

all: cargo-build $(SUBDIRS)

.PHONY: $(SUBDIRS)
$(SUBDIRS):
	$(MAKE) -C $@

test:
	#cargo test test_broadcast_future
	#cargo test $(CARGO_BUILD_ARGS)
	$(CARGO) test $(tests) $(CARGO_BUILD_ARGS)

doc:
	$(CARGO) doc --workspace --no-deps $(CARGO_BUILD_ARGS)

# always re-create this dir
.PHONY: build
build:
	rm -rf build
	mkdir build
	cp -a debian \
	  Cargo.toml build.rs src \
	  $(SUBCRATES) \
	  docs etc examples tests www zsh-completions \
	  defines.mk Makefile \
	  ./build/
	rm -f build/Cargo.lock
	find build/debian -name "*.hint" -delete
	$(foreach i,$(SUBDIRS), \
	    $(MAKE) -C build/$(i) clean ;)


.PHONY: proxmox-backup-docs
$(DOC_DEB) $(DEBS): proxmox-backup-docs
proxmox-backup-docs: build
	cd build; dpkg-buildpackage -b -us -uc --no-pre-clean
	lintian $(DOC_DEB)

# copy the local target/ dir as a build-cache
.PHONY: deb
$(DEBS): deb
deb: build
	cd build; dpkg-buildpackage -b -us -uc --no-pre-clean --build-profiles=nodoc
	lintian $(DEBS)

.PHONY: deb-all
deb-all: build
	cd build; dpkg-buildpackage -b -us -uc --no-pre-clean
	lintian $(DEBS) $(DOC_DEB)

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
dinstall: ${SERVER_DEB} ${SERVER_DBG_DEB} ${CLIENT_DEB} ${CLIENT_DBG_DEB}
	dpkg -i $^

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
	install -dm755 $(DESTDIR)$(ZSH_COMPL_DEST)
	$(foreach i,$(USR_BIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(BINDIR)/ ; \
	    install -m644 zsh-completions/_$(i) $(DESTDIR)$(ZSH_COMPL_DEST)/ ;)
	install -dm755 $(DESTDIR)$(SBINDIR)
	$(foreach i,$(USR_SBIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(SBINDIR)/ ; \
	    install -m644 zsh-completions/_$(i) $(DESTDIR)$(ZSH_COMPL_DEST)/ ;)
	install -dm755 $(DESTDIR)$(LIBEXECDIR)/proxmox-backup
	install -dm755 $(DESTDIR)$(LIBEXECDIR)/proxmox-backup/file-restore
	$(foreach i,$(RESTORE_BIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(LIBEXECDIR)/proxmox-backup/file-restore/ ;)
	# install sg-tape-cmd as setuid binary
	install -m4755 -o root -g root $(COMPILEDIR)/sg-tape-cmd $(DESTDIR)$(LIBEXECDIR)/proxmox-backup/sg-tape-cmd
	$(foreach i,$(SERVICE_BIN), \
	    install -m755 $(COMPILEDIR)/$(i) $(DESTDIR)$(LIBEXECDIR)/proxmox-backup/ ;)
	$(MAKE) -C www install
	$(MAKE) -C docs install

.PHONY: upload
upload: ${SERVER_DEB} ${CLIENT_DEB} ${RESTORE_DEB} ${DOC_DEB}
	# check if working directory is clean
	git diff --exit-code --stat && git diff --exit-code --stat --staged
	tar cf - ${SERVER_DEB} ${SERVER_DBG_DEB} ${DOC_DEB} ${CLIENT_DEB} ${CLIENT_DBG_DEB} | \
	    ssh -X repoman@repo.proxmox.com upload --product pbs --dist bullseye
	tar cf - ${CLIENT_DEB} ${CLIENT_DBG_DEB} | ssh -X repoman@repo.proxmox.com upload --product "pve,pmg,pbs-client" --dist bullseye
	tar cf - ${RESTORE_DEB} ${RESTORE_DBG_DEB} | ssh -X repoman@repo.proxmox.com upload --product "pve" --dist bullseye
