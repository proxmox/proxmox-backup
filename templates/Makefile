include ../defines.mk

NOTIFICATION_TEMPLATES=					\
	default/test-body.txt.hbs			\
	default/test-body.html.hbs			\
	default/test-subject.txt.hbs		\

all:

clean:

install:
	install -dm755 $(DESTDIR)$(DATAROOTDIR)/proxmox-backup/templates/default
	$(foreach i,$(NOTIFICATION_TEMPLATES), \
	    install -m644 $(i) $(DESTDIR)$(DATAROOTDIR)/proxmox-backup/templates/$(i) ;)
