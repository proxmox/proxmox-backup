include ../defines.mk

GENERATED_SYNOPSIS := 						\
	proxmox-backup-client/synopsis.rst			\
	proxmox-backup-client/catalog-shell-synopsis.rst 	\
	proxmox-backup-manager/synopsis.rst			\
	pxar/synopsis.rst					\
	backup-protocol-api.rst					\
	reader-protocol-api.rst

MANUAL_PAGES := 			\
	pxar.1				\
	proxmox-backup-proxy.1		\
	proxmox-backup-client.1		\
	proxmox-backup-manager.1


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

all: ${MANUAL_PAGES}

# Extract backup protocol docs
backup-protocol-api.rst: ${COMPILEDIR}/dump-backup-api
	${COMPILEDIR}/dump-backup-api >$@

# Extract reader protocol docs
reader-protocol-api.rst: ${COMPILEDIR}/dump-reader-api
	${COMPILEDIR}/dump-backup-api >$@

# Build manual pages using rst2man

pxar/synopsis.rst: ${COMPILEDIR}/pxar
	${COMPILEDIR}/pxar printdoc > pxar/synopsis.rst

pxar.1: pxar/man1.rst  pxar/description.rst pxar/synopsis.rst
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

.PHONY: onlinehelpinfo
onlinehelpinfo:
	@echo "Generating OnlineHelpInfo.js..."
	$(SPHINXBUILD) -b proxmox-scanrefs $(ALLSPHINXOPTS) $(BUILDDIR)/scanrefs
	@echo "Build finished. OnlineHelpInfo.js is in $(BUILDDIR)/scanrefs."

.PHONY: html
html: ${GENERATED_SYNOPSIS}
	$(SPHINXBUILD) -b html $(ALLSPHINXOPTS) $(BUILDDIR)/html
	cp images/proxmox-logo.svg $(BUILDDIR)/html/_static/
	cp custom.css $(BUILDDIR)/html/_static/
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
	rm -r -f *~ *.1 ${BUILDDIR} ${GENERATED_SYNOPSIS}


install_manual_pages: ${MANUAL_PAGES}
	install -dm755 $(DESTDIR)$(MAN1DIR)
	for i in ${MANUAL_PAGES}; do install -m755 $$i $(DESTDIR)$(MAN1DIR)/ ; done

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
