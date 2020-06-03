Ext.define('pbs-sync-jobs-status', {
    extend: 'Ext.data.Model',
    fields: [
	'id', 'remote', 'remote-store', 'store', 'schedule',
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
	url: '/api2/json/admin/sync',
    },
});

Ext.define('PBS.config.SyncJobView', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsSyncJobView',

    stateful: true,
    stateId: 'grid-sync-jobs',

    title: gettext('Sync Jobs'),

    controller: {
	xclass: 'Ext.app.ViewController',

	addSyncJob: function() {
	    let me = this;
            Ext.create('PBS.window.SyncJobEdit', {
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
            }).show();
	},

	editSyncJob: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

            Ext.create('PBS.window.SyncJobEdit', {
                id: selection[0].data.id,
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
		upid
	    }).show();
	},

	runSyncJob: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

	    let id = selection[0].data.id;
	    Proxmox.Utils.API2Request({
		method: 'POST',
		url: `/admin/sync/${id}/run`,
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

	render_sync_status: function(value, metadata, record) {
	    if (!record.data['last-run-upid']) {
		return '-';
	    }

	    if (!record.data['last-run-endtime']) {
		metadata.tdCls = 'x-grid-row-loading';
		return '';
	    }

	    if (value === 'OK') {
		return `<i class="fa fa-check good"></i> ${gettext("OK")}`;
	    }

	    return `<i class="fa fa-times critical"></i> ${gettext("Error")}:${value}`;
	},

	render_next_run: function(value, metadat, record) {
	    if (!value) return '-';

	    let now = new Date();
	    let next = new Date(value*1000);

	    if (next < now) {
		return gettext('pending');
	    }
	    return Proxmox.Utils.render_timestamp(value);
	},

	render_optional_timestamp: function(value, metadata, record) {
	    if (!value) return '-';
	    return Proxmox.Utils.render_timestamp(value);
	},

	reload: function() { this.getView().getStore().rstore.load(); },

	init: function(view) {
	    Proxmox.Utils.monStoreErrors(view, view.getStore().rstore);
	},
    },

    listeners: {
	activate: 'reload',
	itemdblclick: 'editSyncJob',
    },

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: 'id',
	rstore: {
	    type: 'update',
	    storeid: 'pbs-sync-jobs-status',
	    model: 'pbs-sync-jobs-status',
	    autoStart: true,
	    interval: 5000,
	},
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Add'),
	    handler: 'addSyncJob',
	    selModel: false,
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Edit'),
	    handler: 'editSyncJob',
	    disabled: true,
	},
	{
	    xtype: 'proxmoxStdRemoveButton',
	    baseurl: '/config/sync/',
	    callback: 'reload',
	},
	'-',
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Log'),
	    handler: 'openTaskLog',
	    enableFn: (rec) => !!rec.data['last-run-upid'],
	    disabled: true,
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Run now'),
	    handler: 'runSyncJob',
	    disabled: true,
	},
    ],

    viewConfig: {
	trackOver: false,
    },

    columns: [
	{
	    header: gettext('Sync Job'),
	    width: 200,
	    sortable: true,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'id',
	},
	{
	    header: gettext('Remote'),
	    width: 200,
	    sortable: true,
	    dataIndex: 'remote',
	},
	{
	    header: gettext('Remote Store'),
	    width: 200,
	    sortable: true,
	    dataIndex: 'remote-store',
	},
	{
	    header: gettext('Local Store'),
	    width: 200,
	    sortable: true,
	    dataIndex: 'store',
	},
	{
	    header: gettext('Schedule'),
	    sortable: true,
	    dataIndex: 'schedule',
	},
	{
	    header: gettext('Status'),
	    dataIndex: 'last-run-state',
	    flex: 1,
	    renderer: 'render_sync_status',
	},
	{
	    header: gettext('Last Sync'),
	    sortable: true,
	    minWidth: 200,
	    renderer: 'render_optional_timestamp',
	    dataIndex: 'last-run-endtime',
	},
	{
	    text: gettext('Duration'),
	    dataIndex: 'duration',
	    width: 60,
	    renderer: Proxmox.Utils.render_duration,
	},
	{
	    header: gettext('Next Run'),
	    sortable: true,
	    minWidth: 200,
	    renderer: 'render_next_run',
	    dataIndex: 'next-run',
	},
	{
	    header: gettext('Comment'),
	    hidden: true,
	    sortable: true,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'comment',
	},
    ],
});
