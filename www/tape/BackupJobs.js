Ext.define('pbs-tape-backup-job-status', {
    extend: 'Ext.data.Model',
    fields: [
	'id', 'store', 'pool', 'drive', 'store', 'schedule', 'comment',
	{ name: 'eject-media', type: 'boolean' },
	{ name: 'export-media-set', type: 'boolean' },
	'next-run', 'last-run-upid', 'last-run-state', 'last-run-endtime',
	{
	    name: 'duration',
	    calculate: function(data) {
		let endtime = data['last-run-endtime'];
		if (!endtime) return undefined;
		let task = Proxmox.Utils.parse_task_upid(data['last-run-upid']);
		return endtime - task.starttime;
	    },
	},
    ],
    idProperty: 'id',
    proxy: {
	type: 'proxmox',
	url: '/api2/json/tape/backup',
    },
});

Ext.define('PBS.config.TapeBackupJobView', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsTapeBackupJobView',

    stateful: true,
    stateId: 'grid-tape-backup-jobs-v1',

    title: gettext('Tape Backup Jobs'),

    controller: {
	xclass: 'Ext.app.ViewController',

	startStore: function() { this.getView().getStore().rstore.startUpdate(); },

	stopStore: function() { this.getView().getStore().rstore.stopUpdate(); },

	reload: function() { this.getView().getStore().rstore.load(); },

	init: function(view) {
	    Proxmox.Utils.monStoreErrors(view, view.getStore().rstore);
	},
    },

    listeners: {
	activate: 'startStore',
	deactivate: 'stopStore',
	//itemdblclick: 'editSyncJob',
    },

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: 'id',
	rstore: {
	    type: 'update',
	    storeid: 'pbs-tape-backup-job-status',
	    model: 'pbs-tape-backup-job-status',
	    interval: 5000,
	},
    },

    viewConfig: {
	trackOver: false,
    },

    columns: [
	{
	    header: gettext('Job ID'),
	    dataIndex: 'id',
	    renderer: Ext.String.htmlEncode,
	    maxWidth: 220,
	    minWidth: 75,
	    flex: 1,
	    sortable: true,
	},
	{
	    header: gettext('Datastore'),
	    dataIndex: 'store',
	    width: 120,
	    sortable: true,
	},
	{
	    header: gettext('Media Pool'),
	    dataIndex: 'pool',
	    width: 120,
	    sortable: true,
	},
	{
	    header: gettext('Drive'),
	    dataIndex: 'drive',
	    width: 120,
	    sortable: true,
	},
	{
	    header: gettext('Schedule'),
	    dataIndex: 'schedule',
	    maxWidth: 220,
	    minWidth: 80,
	    flex: 1,
	    sortable: true,
	},
	{
	    header: gettext('Last Backup'),
	    dataIndex: 'last-run-endtime',
	    renderer: PBS.Utils.render_optional_timestamp,
	    width: 150,
	    sortable: true,
	},
	{
	    text: gettext('Duration'),
	    dataIndex: 'duration',
	    renderer: Proxmox.Utils.render_duration,
	    width: 80,
	},
	{
	    header: gettext('Status'),
	    dataIndex: 'last-run-state',
	    renderer: PBS.Utils.render_task_status,
	    flex: 3,
	},
	{
	    header: gettext('Next Run'),
	    dataIndex: 'next-run',
	    renderer: PBS.Utils.render_next_task_run,
	    width: 150,
	    sortable: true,
	},
	{
	    header: gettext('Comment'),
	    dataIndex: 'comment',
	    renderer: Ext.String.htmlEncode,
	    flex: 2,
	    sortable: true,
	},
    ],

    initComponent: function() {
	let me = this;

	me.callParent();
    },
});
