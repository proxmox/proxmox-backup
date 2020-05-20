include ../defines.mk

IMAGES := \
	images/logo-128.png \
	images/proxmox_logo.png

JSSRC=							\
	form/UserSelector.js				\
	config/UserView.js				\
	window/UserEdit.js				\
	Utils.js					\
	LoginView.js					\
	VersionInfo.js					\
	SystemConfiguration.js				\
	Subscription.js					\
	DataStorePrune.js				\
	DataStoreConfig.js				\
	DataStoreStatus.js				\
	DataStoreContent.js				\
	ServerStatus.js					\
	ServerAdministration.js				\
	Dashboard.js					\
	NavigationTree.js				\
	Application.js					\
	MainView.js

all: js/proxmox-backup-gui.js css/ext6-pbs.css

js:
	mkdir js

js/proxmox-backup-gui.js: js OnlineHelpInfo.js ${JSSRC}
	cat OnlineHelpInfo.js ${JSSRC} >$@.tmp
	mv $@.tmp $@

.PHONY: clean
clean:
	find . -name '*~' -exec rm {} ';'	
	rm -rf js

install: js/proxmox-backup-gui.js css/ext6-pbs.css index.hbs
	install -dm755 $(DESTDIR)$(JSDIR)
	install -m644 index.hbs $(DESTDIR)$(JSDIR)/
	install -dm755 $(DESTDIR)$(JSDIR)/js
	install -m644 js/proxmox-backup-gui.js $(DESTDIR)$(JSDIR)/js/
	install -dm755 $(DESTDIR)$(JSDIR)/css
	install -m644 css/ext6-pbs.css $(DESTDIR)$(JSDIR)/css/
	install -dm755 $(DESTDIR)$(JSDIR)/images
	$(foreach i,$(IMAGES), \
	    install -m644 $(i) $(DESTDIR)$(JSDIR)/images/ ;)
