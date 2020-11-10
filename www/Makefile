include ../defines.mk

IMAGES := \
	images/logo-128.png \
	images/proxmox_logo.png

JSSRC=							\
	Utils.js					\
	form/UserSelector.js				\
	form/TokenSelector.js				\
	form/AuthidSelector.js				\
	form/RemoteSelector.js				\
	form/DataStoreSelector.js			\
	form/CalendarEvent.js				\
	form/PermissionPathSelector.js			\
	data/RunningTasksStore.js			\
	button/TaskButton.js				\
	config/UserView.js				\
	config/TokenView.js				\
	config/RemoteView.js				\
	config/ACLView.js				\
	config/SyncView.js				\
	config/VerifyView.js				\
	window/ACLEdit.js				\
	window/BackupFileDownloader.js			\
	window/BackupGroupChangeOwner.js		\
	window/CreateDirectory.js			\
	window/DataStoreEdit.js				\
	window/FileBrowser.js				\
	window/NotesEdit.js				\
	window/RemoteEdit.js				\
	window/NotifyOptions.js				\
	window/SyncJobEdit.js				\
	window/UserEdit.js				\
	window/UserPassword.js				\
	window/TokenEdit.js				\
	window/VerifyJobEdit.js				\
	window/ZFSCreate.js				\
	dashboard/DataStoreStatistics.js		\
	dashboard/LongestTasks.js			\
	dashboard/RunningTasks.js			\
	dashboard/TaskSummary.js			\
	panel/Tasks.js					\
	panel/XtermJsConsole.js				\
	panel/AccessControl.js				\
	panel/StorageAndDisks.js			\
	panel/UsageChart.js				\
	ZFSList.js					\
	DirectoryList.js				\
	LoginView.js					\
	VersionInfo.js					\
	SystemConfiguration.js				\
	Subscription.js					\
	datastore/Summary.js				\
	datastore/Notes.js				\
	datastore/PruneAndGC.js				\
	datastore/Prune.js				\
	datastore/Content.js				\
	datastore/OptionView.js				\
	datastore/Panel.js				\
	datastore/DataStoreListSummary.js		\
	datastore/DataStoreList.js			\
	ServerStatus.js					\
	ServerAdministration.js				\
	Dashboard.js					\
	NavigationTree.js				\
	Application.js					\
	MainView.js

all: js/proxmox-backup-gui.js css/ext6-pbs.css

js:
	mkdir js

.PHONY: OnlineHelpInfo.js
OnlineHelpInfo.js:
	$(MAKE) -C ../docs onlinehelpinfo
	mv ../docs/output/scanrefs/OnlineHelpInfo.js .

js/proxmox-backup-gui.js: .lint-incremental js OnlineHelpInfo.js ${JSSRC}
	cat OnlineHelpInfo.js ${JSSRC} >$@.tmp
	mv $@.tmp $@

.PHONY: check
check:
	eslint ${JSSRC}
	touch ".lint-incremental"

.lint-incremental: ${JSSRC}
	eslint $?
	touch "$@"

.PHONY: clean
clean:
	find . -name '*~' -exec rm {} ';'
	rm -rf js .lint-incremental

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
