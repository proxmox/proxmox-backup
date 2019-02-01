include ../defines.mk

UNITS := \
	proxmox-backup.service \
	proxmox-backup-proxy.service

all: $(UNITS)

clean:
	rm -f $(UNITS)

.SUFFIXES: .service.in .service
.service.in.service:
	sed \
	    -e 's!%LIBEXECDIR%!$(LIBEXECDIR)!g' \
	    -e 's!%PROXY_USER%!$(PROXY_USER)!g' \
	    $< >$@.tmp
	mv $@.tmp $@
