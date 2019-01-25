JSSRC=							\
	Utils.js					\
	Logo.js						\
	VersionInfo.js					\
	SystemConfiguration.js				\
	Subscription.js					\
	DataStoreConfig.js				\
	NavigationTree.js				\
	Application.js					\
	MainView.js

all: proxmox-backup-gui.js

proxmox-backup-gui.js: OnlineHelpInfo.js ${JSSRC}
	cat OnlineHelpInfo.js ${JSSRC} >$@.tmp
	mv $@.tmp $@

.PHONY: clean
clean:
	find . -name '*~' -exec rm {} ';'	
	rm -f proxmox-backup-gui.js

