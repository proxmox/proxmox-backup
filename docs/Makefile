include ../defines.mk

GENERATED_SYNOPSIS := 						\
	proxmox-tape/synopsis.rst				\
	proxmox-backup-client/synopsis.rst			\
	proxmox-backup-client/catalog-shell-synopsis.rst 	\
	proxmox-backup-manager/synopsis.rst			\
	proxmox-file-restore/synopsis.rst			\
	pxar/synopsis.rst					\
	pmtx/synopsis.rst					\
	pmt/synopsis.rst					\
	config/media-pool/config.rst				\
	config/tape/config.rst					\
	config/tape-job/config.rst				\
	config/user/config.rst					\
	config/remote/config.rst				\
	config/sync/config.rst					\
	config/verification/config.rst				\
	config/acl/roles.rst					\
	config/datastore/config.rst

MAN1_PAGES := 				\
	pxar.1				\
	pmtx.1				\
	pmt.1				\
	proxmox-tape.1			\
	proxmox-backup-proxy.1		\
	proxmox-backup-client.1		\
	proxmox-backup-manager.1	\
	proxmox-file-restore.1

MAN5_PAGES :=				\
	media-pool.cfg.5		\
	tape.cfg.5			\
	tape-job.cfg.5			\
	acl.cfg.5			\
	user.cfg.5			\
	remote.cfg.5			\
	sync.cfg.5			\
	verification.cfg.5		\
	datastore.cfg.5

PRUNE_SIMULATOR_FILES := 					\
	prune-simulator/index.html				\
	prune-simulator/documentation.html			\
	prune-simulator/clear-trigger.png			\
	prune-simulator/prune-simulator.js

PRUNE_SIMULATOR_JS_SOURCE :=						\
	/usr/share/javascript/proxmox-widget-toolkit-dev/Toolkit.js	\
	prune-simulator/prune-simulator_source.js

LTO_BARCODE_JS_SOURCE :=						\
	/usr/share/javascript/proxmox-widget-toolkit-dev/Toolkit.js	\
	lto-barcode/code39.js						\
	lto-barcode/prefix-field.js					\
	lto-barcode/label-style.js					\
	lto-barcode/tape-type.js					\
	lto-barcode/paper-size.js					\
	lto-barcode/page-layout.js					\
	lto-barcode/page-calibration.js					\
	lto-barcode/label-list.js					\
	lto-barcode/label-setup.js					\
	lto-barcode/lto-barcode.js

LTO_BARCODE_FILES :=						\
	lto-barcode/index.html					\
	lto-barcode/lto-barcode-generator.js

API_VIEWER_SOURCES=				\
	api-viewer/index.html			\
	api-viewer/apidoc.js

# Sphinx documentation setup
SPHINXOPTS    =
SPHINXBUILD   = sphinx-build
BUILDDIR      = output

ifeq ($(BUILD_MODE), release)
COMPILEDIR := ../target/release
SPHINXOPTS    += -t release
else
COMPILEDIR := ../target/debug
SPHINXOPTS    += -t devbuild
endif

# Sphinx internal variables.
ALLSPHINXOPTS   = -d $(BUILDDIR)/doctrees $(SPHINXOPTS) .

all: ${MAN1_PAGES} ${MAN5_PAGES}

# Build manual pages using rst2man

pxar/synopsis.rst: ${COMPILEDIR}/pxar
	${COMPILEDIR}/pxar printdoc > pxar/synopsis.rst

pxar.1: pxar/man1.rst  pxar/description.rst pxar/synopsis.rst
	rst2man $< >$@


pmtx/synopsis.rst: ${COMPILEDIR}/pmtx
	${COMPILEDIR}/pmtx printdoc > pmtx/synopsis.rst

pmtx.1: pmtx/man1.rst  pmtx/description.rst pmtx/synopsis.rst
	rst2man $< >$@


pmt/synopsis.rst: ${COMPILEDIR}/pmt
	${COMPILEDIR}/pmt printdoc > pmt/synopsis.rst

pmt.1: pmt/man1.rst  pmt/description.rst pmt/options.rst pmt/synopsis.rst
	rst2man $< >$@

config/datastore/config.rst: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen datastore.cfg >$@

datastore.cfg.5: config/datastore/man5.rst config/datastore/config.rst config/datastore/format.rst
	rst2man $< >$@

config/user/config.rst: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen user.cfg >$@

user.cfg.5: config/user/man5.rst config/user/config.rst config/user/format.rst
	rst2man $< >$@

config/remote/config.rst: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen remote.cfg >$@

remote.cfg.5: config/remote/man5.rst config/remote/config.rst config/remote/format.rst
	rst2man $< >$@

config/sync/config.rst: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen sync.cfg >$@

sync.cfg.5: config/sync/man5.rst config/sync/config.rst config/sync/format.rst
	rst2man $< >$@

config/verification/config.rst: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen verification.cfg >$@

verification.cfg.5: config/verification/man5.rst config/verification/config.rst config/verification/format.rst
	rst2man $< >$@

config/acl/roles.rst: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen "config::acl::Role" >$@

acl.cfg.5: config/acl/man5.rst config/acl/roles.rst config/acl/format.rst
	rst2man $< >$@

config/media-pool/config.rst: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen media-pool.cfg >$@

media-pool.cfg.5: config/media-pool/man5.rst config/media-pool/config.rst config/media-pool/format.rst
	rst2man $< >$@

config/tape/config.rst: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen tape.cfg >$@

tape.cfg.5: config/tape/man5.rst config/tape/config.rst config/tape/format.rst
	rst2man $< >$@

config/tape-job/config.rst: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen tape-job.cfg >$@

tape-job.cfg.5: config/tape-job/man5.rst config/tape-job/config.rst config/tape-job/format.rst
	rst2man $< >$@

proxmox-tape/synopsis.rst: ${COMPILEDIR}/proxmox-tape
	${COMPILEDIR}/proxmox-tape printdoc > proxmox-tape/synopsis.rst

proxmox-tape.1: proxmox-tape/man1.rst proxmox-tape/description.rst proxmox-tape/synopsis.rst
	rst2man $< >$@

proxmox-backup-client/synopsis.rst: ${COMPILEDIR}/proxmox-backup-client
	${COMPILEDIR}/proxmox-backup-client printdoc > proxmox-backup-client/synopsis.rst

proxmox-backup-client/catalog-shell-synopsis.rst: ${COMPILEDIR}/dump-catalog-shell-cli
	${COMPILEDIR}/dump-catalog-shell-cli > proxmox-backup-client/catalog-shell-synopsis.rst

proxmox-backup-client.1: proxmox-backup-client/man1.rst  proxmox-backup-client/description.rst proxmox-backup-client/synopsis.rst  proxmox-backup-client/catalog-shell-synopsis.rst
	rst2man $< >$@

proxmox-backup-manager/synopsis.rst: ${COMPILEDIR}/proxmox-backup-manager
	${COMPILEDIR}/proxmox-backup-manager printdoc > proxmox-backup-manager/synopsis.rst

proxmox-backup-manager.1: proxmox-backup-manager/man1.rst  proxmox-backup-manager/description.rst proxmox-backup-manager/synopsis.rst
	rst2man $< >$@

proxmox-backup-proxy.1: proxmox-backup-proxy/man1.rst  proxmox-backup-proxy/description.rst
	rst2man $< >$@

proxmox-file-restore/synopsis.rst: ${COMPILEDIR}/proxmox-file-restore
	${COMPILEDIR}/proxmox-file-restore printdoc > proxmox-file-restore/synopsis.rst

proxmox-file-restore.1: proxmox-file-restore/man1.rst  proxmox-file-restore/description.rst proxmox-file-restore/synopsis.rst
	rst2man $< >$@

.PHONY: onlinehelpinfo
onlinehelpinfo:
	@echo "Generating OnlineHelpInfo.js..."
	$(SPHINXBUILD) -b proxmox-scanrefs -Q $(ALLSPHINXOPTS) $(BUILDDIR)/scanrefs
	@echo "Build finished. OnlineHelpInfo.js is in $(BUILDDIR)/scanrefs."

api-viewer/apidata.js: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen apidata.js >$@

api-viewer/apidoc.js: api-viewer/apidata.js api-viewer/PBSAPI.js
	cat api-viewer/apidata.js api-viewer/PBSAPI.js >$@

prune-simulator/prune-simulator.js: ${PRUNE_SIMULATOR_JS_SOURCE}
	cat ${PRUNE_SIMULATOR_JS_SOURCE} >$@.tmp
	mv $@.tmp $@

lto-barcode/lto-barcode-generator.js: ${LTO_BARCODE_JS_SOURCE}
	cat ${LTO_BARCODE_JS_SOURCE} >$@.tmp
	mv $@.tmp $@

.PHONY: html
html: ${GENERATED_SYNOPSIS} images/proxmox-logo.svg custom.css conf.py ${PRUNE_SIMULATOR_FILES} ${LTO_BARCODE_FILES} ${API_VIEWER_SOURCES}
	$(SPHINXBUILD) -b html $(ALLSPHINXOPTS) $(BUILDDIR)/html
	install -m 0644 custom.js custom.css images/proxmox-logo.svg $(BUILDDIR)/html/_static/
	install -dm 0755 $(BUILDDIR)/html/prune-simulator
	install -m 0644 ${PRUNE_SIMULATOR_FILES} $(BUILDDIR)/html/prune-simulator
	install -dm 0755 $(BUILDDIR)/html/lto-barcode
	install -m 0644 ${LTO_BARCODE_FILES} $(BUILDDIR)/html/lto-barcode
	install -dm 0755 $(BUILDDIR)/html/api-viewer
	install -m 0644 ${API_VIEWER_SOURCES} $(BUILDDIR)/html/api-viewer
	@echo
	@echo "Build finished. The HTML pages are in $(BUILDDIR)/html."

.PHONY: latexpdf
latexpdf: ${GENERATED_SYNOPSIS}
	@echo "Requires python3-sphinx, texlive-xetex, xindy and texlive-fonts-extra"
	$(SPHINXBUILD) -b latex $(ALLSPHINXOPTS) $(BUILDDIR)/latex
	@echo "Running LaTeX files through xelatex..."
	$(MAKE) -C $(BUILDDIR)/latex all-pdf
	@echo "xelatex finished; the PDF files are in $(BUILDDIR)/latex."

.PHONY: epub3
epub3: ${GENERATED_SYNOPSIS}
	$(SPHINXBUILD) -b epub3 $(ALLSPHINXOPTS) $(BUILDDIR)/epub3
	@echo
	@echo "Build finished. The epub3 file is in $(BUILDDIR)/epub3."

clean:
	rm -r -f *~ *.1 ${BUILDDIR} ${GENERATED_SYNOPSIS} api-viewer/apidata.js


install_manual_pages: ${MAN1_PAGES} ${MAN5_PAGES}
	install -dm755 $(DESTDIR)$(MAN1DIR)
	for i in ${MAN1_PAGES}; do install -m755 $$i $(DESTDIR)$(MAN1DIR)/ ; done
	install -dm755 $(DESTDIR)$(MAN5DIR)
	for i in ${MAN5_PAGES}; do install -m755 $$i $(DESTDIR)$(MAN5DIR)/ ; done

install_html: html
	install -dm755 $(DESTDIR)$(DOCDIR)
	rsync -a ${BUILDDIR}/html $(DESTDIR)$(DOCDIR)

install_pdf: latexpdf
	install -dm755 $(DESTDIR)$(DOCDIR)
	install -m 0644 output/latex/ProxmoxBackup.pdf $(DESTDIR)$(DOCDIR)/proxmox-backup.pdf

ifneq ($(filter nodoc,$(DEB_BUILD_PROFILES)),)

install: install_manual_pages

else

install: install_manual_pages install_html install_pdf

endif
