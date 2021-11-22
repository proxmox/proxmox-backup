Ext.define('pbs-tape-backup-job-status', {
    extend: 'Ext.data.Model',
    fields: [
	'id', 'store', 'pool', 'drive', 'store', 'schedule', 'comment', 'group-filter',
	{ name: 'eject-media', type: 'boolean' },
	{ name: 'export-media-set', type: 'boolean' },
	{ name: 'latest-only', type: 'boolean' },
	'next-run', 'next-media-label', 'last-run-upid', 'last-run-state', 'last-run-endtime',
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

	addJob: function() {
	    let me = this;
	    Ext.create('PBS.TapeManagement.BackupJobEdit', {
		autoShow: true,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	editJob: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (!selection || selection.length < 1) {
		return;
	    }

	    Ext.create('PBS.TapeManagement.BackupJobEdit', {
		id: selection[0].data.id,
		autoShow: true,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	openTaskLog: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

	    let upid = selection[0].data['last-run-upid'];
	    if (!upid) return;

	    Ext.create('Proxmox.window.TaskViewer', {
		upid,
	    }).show();
	},

	runJob: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

	    let id = selection[0].data.id;
	    Proxmox.Utils.API2Request({
		method: 'POST',
		url: `/tape/backup/${id}`,
		success: function(response, opt) {
		    Ext.create('Proxmox.window.TaskViewer', {
		        upid: response.result.data,
		        taskDone: function(success) {
			    me.reload();
		        },
		    }).show();
		},
		failure: function(response, opt) {
		    Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		},
	    });
	},

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
	itemdblclick: 'editJob',
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

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Add'),
	    selModel: false,
	    handler: 'addJob',
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Edit'),
	    handler: 'editJob',
	    disabled: true,
	},
	{
	    xtype: 'proxmoxStdRemoveButton',
	    baseurl: '/config/tape-backup-job/',
	    confirmMsg: gettext('Remove entry?'),
	    callback: 'reload',
	},
	'-',
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Show Log'),
	    handler: 'openTaskLog',
	    enableFn: (rec) => !!rec.data['last-run-upid'],
	    disabled: true,
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Run now'),
	    handler: 'runJob',
	    disabled: true,
	},
    ],

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
	    header: gettext('Eject'),
	    dataIndex: 'eject-media',
	    renderer: Proxmox.Utils.format_boolean,
	    width: 60,
	    sortable: false,
	},
	{
	    header: gettext('Export'),
	    dataIndex: 'export-media-set',
	    renderer: Proxmox.Utils.format_boolean,
	    width: 60,
	    sortable: false,
	},
	{
	    header: gettext('Latest Only'),
	    dataIndex: 'latest-only',
	    renderer: Proxmox.Utils.format_boolean,
	    sortable: false,
	},
	{
	    header: gettext('Backup Groups'),
	    dataIndex: 'group-filter',
	    renderer: v => v ? Ext.String.htmlEncode(v) : gettext('All'),
	    width: 80,
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
	    header: gettext('Next Media'),
	    dataIndex: 'next-media-label',
	    width: 100,
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
