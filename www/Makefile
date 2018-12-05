JSSRC=							\
	Utils.js					\
	Logo.js						\
	VersionInfo.js					\
	Application.js					\
	MainView.js

all: proxmox-backup-gui.js

proxmox-backup-gui.js: ${JSSRC}
	cat ${JSSRC} >$@.tmp
	mv $@.tmp $@

.PHONY: clean
clean:
	find . -name '*~' -exec rm {} ';'	
	rm -r proxmox-backup-gui.js

