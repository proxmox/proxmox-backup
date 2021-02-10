PREFIX = /usr
BINDIR = $(PREFIX)/bin
SBINDIR = $(PREFIX)/sbin
LIBDIR = $(PREFIX)/lib
LIBEXECDIR = $(LIBDIR)
DATAROOTDIR = $(PREFIX)/share
MAN1DIR = $(PREFIX)/share/man/man1
MAN5DIR = $(PREFIX)/share/man/man5
DOCDIR = $(PREFIX)/share/doc/proxmox-backup
JSDIR = $(DATAROOTDIR)/javascript/proxmox-backup
SYSCONFDIR = /etc
ZSH_COMPL_DEST = $(PREFIX)/share/zsh/vendor-completions

# For local overrides
-include local.mak
