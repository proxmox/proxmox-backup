PACKAGE := proxmox-backup
PKGVER := 0.1
PKGREL := 1

PREFIX := /usr
BINDIR := $(PREFIX)/bin
SBINDIR := $(PREFIX)/sbin
LIBDIR := $(PREFIX)/lib
LIBEXECDIR := $(LIBDIR)
DATAROOTDIR := $(PREFIX)/share
JSDIR := $(DATAROOTDIR)/javascript/proxmox-backup
SYSCONFDIR := /etc

# For local overrides
-include local.mak
