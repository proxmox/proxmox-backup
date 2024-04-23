include ../defines.mk

NOTIFICATION_TEMPLATES=						\
	default/acme-err-body.txt.hbs			\
	default/acme-err-subject.txt.hbs		\
	default/gc-err-body.txt.hbs				\
	default/gc-ok-body.txt.hbs				\
	default/gc-err-subject.txt.hbs			\
	default/gc-ok-subject.txt.hbs			\
	default/package-updates-body.txt.hbs	\
	default/package-updates-subject.txt.hbs	\
	default/prune-err-body.txt.hbs			\
	default/prune-ok-body.txt.hbs			\
	default/prune-err-subject.txt.hbs		\
	default/prune-ok-subject.txt.hbs		\
	default/sync-err-body.txt.hbs			\
	default/sync-ok-body.txt.hbs			\
	default/sync-err-subject.txt.hbs		\
	default/sync-ok-subject.txt.hbs			\
	default/tape-backup-err-body.txt.hbs	\
	default/tape-backup-err-subject.txt.hbs	\
	default/tape-backup-ok-body.txt.hbs		\
	default/tape-backup-ok-subject.txt.hbs	\
	default/tape-load-body.txt.hbs			\
	default/tape-load-subject.txt.hbs		\
	default/test-body.txt.hbs				\
	default/test-body.html.hbs				\
	default/test-subject.txt.hbs			\
	default/verify-err-body.txt.hbs			\
	default/verify-ok-body.txt.hbs			\
	default/verify-err-subject.txt.hbs		\
	default/verify-ok-subject.txt.hbs		\

all:

clean:

install:
	install -dm755 $(DESTDIR)$(DATAROOTDIR)/proxmox-backup/templates/default
	$(foreach i,$(NOTIFICATION_TEMPLATES), \
	    install -m644 $(i) $(DESTDIR)$(DATAROOTDIR)/proxmox-backup/templates/$(i) ;)
