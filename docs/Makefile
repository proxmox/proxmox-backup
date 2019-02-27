include ../defines.mk

ifeq ($(BUILD_MODE), release)
COMPILEDIR := ../target/release
else
COMPILEDIR := ../target/debug
endif

GENERATED_SYNOPSIS := 				\
	proxmox-backup-client/synopsis.rst

MANUAL_PAGES := 			\
	proxmox-backup-client.1


# Sphinx documentation setup
SPHINXOPTS    =
SPHINXBUILD   = sphinx-build
PAPER         =
BUILDDIR      = output

# Sphinx internal variables.
PAPEROPT_a4     = -D latex_paper_size=a4
PAPEROPT_letter = -D latex_paper_size=letter
ALLSPHINXOPTS   = -d $(BUILDDIR)/doctrees $(PAPEROPT_$(PAPER)) $(SPHINXOPTS) .
# the i18n builder cannot share the environment and doctrees with the others
I18NSPHINXOPTS  = $(PAPEROPT_$(PAPER)) $(SPHINXOPTS) .

all: ${MANUAL_PAGES}

# Build manual pages using rst2man

proxmox-backup-client/synopsis.rst: ${COMPILEDIR}/proxmox-backup-client
	${COMPILEDIR}/proxmox-backup-client printdoc > proxmox-backup-client/synopsis.rst


proxmox-backup-client.1: proxmox-backup-client/man1.rst  proxmox-backup-client/description.rst proxmox-backup-client/synopsis.rst
	rst2man $< >$@

.PHONY: html
html: ${GENERATED_SYNOPSIS}
	$(SPHINXBUILD) -b html $(ALLSPHINXOPTS) $(BUILDDIR)/html
	@echo
	@echo "Build finished. The HTML pages are in $(BUILDDIR)/html."

.PHONY: latexpdf
latexpdf:
	$(SPHINXBUILD) -b latex $(ALLSPHINXOPTS) $(BUILDDIR)/latex
	@echo "Running LaTeX files through pdflatex..."
	$(MAKE) -C $(BUILDDIR)/latex all-pdf
	@echo "pdflatex finished; the PDF files are in $(BUILDDIR)/latex."

.PHONY: epub3
epub3:
	$(SPHINXBUILD) -b epub3 $(ALLSPHINXOPTS) $(BUILDDIR)/epub3
	@echo
	@echo "Build finished. The epub3 file is in $(BUILDDIR)/epub3."

clean:
	rm -r -f *~ *.1 */synopsis.rst ${BUILDDIR}

install: ${MANUAL_PAGES}
	install -dm755 $(DESTDIR)$(MAN1DIR)
	install -m755  proxmox-backup-client.1 $(DESTDIR)$(MAN1DIR)/
