include ../defines.mk

GENERATED_SYNOPSIS := 						\
	proxmox-tape/synopsis.rst				\
	proxmox-backup-client/synopsis.rst			\
	proxmox-backup-client/catalog-shell-synopsis.rst 	\
	proxmox-backup-manager/synopsis.rst			\
	proxmox-backup-debug/synopsis.rst			\
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
	config/datastore/config.rst				\
	config/domains/config.rst

MAN1_PAGES := 				\
	pxar.1				\
	pmtx.1				\
	pmt.1				\
	proxmox-tape.1			\
	proxmox-backup-proxy.1		\
	proxmox-backup-client.1		\
	proxmox-backup-manager.1	\
	proxmox-file-restore.1		\
	proxmox-backup-debug.1		\
	pbs2to3.1			\

MAN5_PAGES :=				\
	media-pool.cfg.5		\
	tape.cfg.5			\
	tape-job.cfg.5			\
	acl.cfg.5			\
	user.cfg.5			\
	remote.cfg.5			\
	sync.cfg.5			\
	verification.cfg.5		\
	datastore.cfg.5			\
	domains.cfg.5

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

API_VIEWER_FILES :=							\
	api-viewer/apidata.js						\
	/usr/share/javascript/proxmox-widget-toolkit-dev/APIViewer.js	\

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

config/%/config.rst: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen $*.cfg >$@

config/acl/roles.rst: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen "config::acl::Role" >$@

%/synopsis.rst: ${COMPILEDIR}/%
	$< printdoc > $@

proxmox-backup-client/catalog-shell-synopsis.rst: ${COMPILEDIR}/dump-catalog-shell-cli
	${COMPILEDIR}/dump-catalog-shell-cli > proxmox-backup-client/catalog-shell-synopsis.rst

${MAN1_PAGES} ${MAN5_PAGES}: man-pages

.PHONY: man-pages
man-pages: ${GENERATED_SYNOPSIS}
	${SPHINXBUILD} ${SPHINXOPTS} -b man ./ ${BUILDDIR}/man

.PHONY: onlinehelpinfo
onlinehelpinfo:
	@echo "Generating OnlineHelpInfo.js..."
	$(SPHINXBUILD) -b proxmox-scanrefs -Q $(ALLSPHINXOPTS) $(BUILDDIR)/scanrefs
	@echo "Build finished. OnlineHelpInfo.js is in $(BUILDDIR)/scanrefs."

api-viewer/apidata.js: ${COMPILEDIR}/docgen
	${COMPILEDIR}/docgen apidata.js >$@

api-viewer/apidoc.js: ${API_VIEWER_FILES}
	cat ${API_VIEWER_FILES} >$@.tmp
	mv $@.tmp $@

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
	rm -f api-viewer/apidoc.js lto-barcode/lto-barcode-generator.js prune-simulator/prune-simulator.js

install_manual_pages: man-pages
	install -dm755 $(DESTDIR)$(MAN1DIR)
	for i in ${MAN1_PAGES}; do install -m755 ${BUILDDIR}/man/$$i $(DESTDIR)$(MAN1DIR)/ ; done
	install -dm755 $(DESTDIR)$(MAN5DIR)
	for i in ${MAN5_PAGES}; do install -m755 ${BUILDDIR}/man/$$i $(DESTDIR)$(MAN5DIR)/ ; done

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
