include ../defines.mk

UNITS := \
	proxmox-backup-daily-update.timer \

DYNAMIC_UNITS := \
	proxmox-backup-banner.service \
	proxmox-backup-daily-update.service \
	proxmox-backup.service \
	proxmox-backup-proxy.service

all: $(UNITS) $(DYNAMIC_UNITS) pbs-enterprise.list

clean:
	rm -f $(DYNAMIC_UNITS)

.SUFFIXES: .service.in .service
.service.in.service:
	sed \
	    -e 's!%LIBEXECDIR%!$(LIBEXECDIR)!g' \
	    -e 's!%PROXY_USER%!$(PROXY_USER)!g' \
	    $< >$@.tmp
	mv $@.tmp $@
