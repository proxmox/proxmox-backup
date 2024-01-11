include ../defines.mk

ESLINT ?= $(if $(shell command -v pve-eslint), pve-eslint, eslint)

IMAGES := \
	images/icon-tape.svg 				\
	images/icon-tape-drive.svg			\
	images/logo-128.png 				\
	images/proxmox_logo.png

TAPE_UI_FILES=						\
	tape/form/AllocationSelector.js			\
	tape/form/ChangerSelector.js			\
	tape/form/DriveSelector.js			\
	tape/form/PoolSelector.js			\
	tape/form/RetentionSelector.js			\
	tape/form/TapeDevicePathSelector.js		\
	tape/form/KeySelector.js			\
	tape/window/ChangerEdit.js			\
	tape/window/DriveEdit.js			\
	tape/window/Erase.js				\
	tape/window/EncryptionEdit.js			\
	tape/window/LabelMedia.js			\
	tape/window/MediaRemoveWindow.js		\
	tape/window/PoolEdit.js				\
	tape/window/TapeBackup.js			\
	tape/window/TapeBackupJob.js			\
	tape/window/TapeRestore.js			\
	tape/BackupOverview.js				\
	tape/ChangerConfig.js				\
	tape/ChangerStatus.js				\
	tape/DriveConfig.js				\
	tape/DriveStatus.js				\
	tape/EncryptionKeys.js				\
	tape/PoolConfig.js				\
	tape/TapeInventory.js				\
	tape/BackupJobs.js				\
	tape/TapeManagement.js

JSSRC=							\
	Utils.js					\
	Schema.js					\
	form/TokenSelector.js				\
	form/AuthidSelector.js				\
	form/RemoteSelector.js				\
	form/RemoteTargetSelector.js   			\
	form/DataStoreSelector.js			\
	form/NamespaceSelector.js			\
	form/NamespaceMaxDepth.js			\
	form/CalendarEvent.js				\
	form/PermissionPathSelector.js			\
	form/GroupSelector.js				\
	form/GroupFilter.js				\
	form/VerifyOutdatedAfter.js			\
	data/RunningTasksStore.js			\
	button/TaskButton.js				\
	panel/PrunePanel.js				\
	config/UserView.js				\
	config/TokenView.js				\
	config/RemoteView.js				\
	config/TrafficControlView.js			\
	config/ACLView.js				\
	config/SyncView.js				\
	config/VerifyView.js				\
	config/PruneView.js				\
	config/WebauthnView.js				\
	config/CertificateView.js			\
	config/NodeOptionView.js			\
	config/MetricServerView.js			\
	window/ACLEdit.js				\
	window/BackupGroupChangeOwner.js		\
	window/CreateDirectory.js			\
	window/DataStoreEdit.js				\
	window/NamespaceEdit.js				\
	window/MaintenanceOptions.js			\
	window/NotesEdit.js				\
	window/RemoteEdit.js				\
	window/TrafficControlEdit.js			\
	window/NotifyOptions.js				\
	window/SyncJobEdit.js				\
	window/PruneJobEdit.js				\
	window/UserEdit.js				\
	window/UserPassword.js				\
	window/Settings.js				\
	window/TokenEdit.js				\
	window/VerifyJobEdit.js				\
	window/VerifyAll.js				\
	window/ZFSCreate.js				\
	window/InfluxDbEdit.js				\
	window/DatastoreRepoInfo.js			\
	dashboard/DataStoreStatistics.js		\
	dashboard/LongestTasks.js			\
	dashboard/RunningTasks.js			\
	dashboard/TaskSummary.js			\
	panel/XtermJsConsole.js				\
	panel/AccessControl.js				\
	panel/StorageAndDisks.js			\
	panel/UsageChart.js				\
	panel/NodeInfo.js				\
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
	NodeNotes.js				        \
	Dashboard.js					\
	${TAPE_UI_FILES}				\
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
	$(ESLINT) --strict ${JSSRC}
	touch ".lint-incremental"

.lint-incremental: ${JSSRC}
	$(ESLINT) $?
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
