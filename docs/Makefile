include ../defines.mk

ifeq ($(BUILD_MODE), release)
COMPILEDIR := ../target/release
else
COMPILEDIR := ../target/debug
endif

##UNITS:= proxmox-backup-client

proxmox-backup-client.1: proxmox-backup-client/man1.rst  proxmox-backup-client/description.rst ${COMPILEDIR}/proxmox-backup-client
	${COMPILEDIR}/proxmox-backup-client printdoc > proxmox-backup-client/synopsis.rst
	rst2man $< >$@

all: proxmox-backup-client.1


clean:
	rm -f *.1 */synopsis.rst

install: proxmox-backup-client.1
	install -dm755 $(DESTDIR)$(MAN1DIR)
	install -m755  proxmox-backup-client.1 $(DESTDIR)$(MAN1DIR)/
