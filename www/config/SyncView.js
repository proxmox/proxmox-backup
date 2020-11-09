Ext.define('pbs-sync-jobs-status', {
    extend: 'Ext.data.Model',
    fields: [
	'id', 'owner', 'remote', 'remote-store', 'store', 'schedule',
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
	'comment',
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
    stateId: 'grid-sync-jobs-v1',

    title: gettext('Sync Jobs'),

    controller: {
	xclass: 'Ext.app.ViewController',

	addSyncJob: function() {
	    let me = this;
	    let view = me.getView();
            Ext.create('PBS.window.SyncJobEdit', {
		datastore: view.datastore,
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
		datastore: view.datastore,
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
		upid,
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

	    let parsed = Proxmox.Utils.parse_task_status(value);
	    let text = value;
	    let icon = '';
	    switch (parsed) {
		case 'unknown':
		    icon = 'question faded';
		    text = Proxmox.Utils.unknownText;
		    break;
		case 'error':
		    icon = 'times critical';
		    text = Proxmox.Utils.errorText + ': ' + value;
		    break;
		case 'warning':
		    icon = 'exclamation warning';
		    break;
		case 'ok':
		    icon = 'check good';
		    text = gettext("OK");
	    }

	    return `<i class="fa fa-${icon}"></i> ${text}`;
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

	render_optional_owner: function(value, metadata, record) {
	    if (!value) return '-';
	    return Ext.String.htmlEncode(value, metadata, record);
	},

	startStore: function() { this.getView().getStore().rstore.startUpdate(); },
	stopStore: function() { this.getView().getStore().rstore.stopUpdate(); },

	reload: function() { this.getView().getStore().rstore.load(); },

	init: function(view) {
	    let params = {};
	    if (view.datastore !== undefined) {
		params.store = view.datastore;
	    }
	    view.getStore().rstore.getProxy().setExtraParams(params);
	    Proxmox.Utils.monStoreErrors(view, view.getStore().rstore);
	},
    },

    listeners: {
	activate: 'startStore',
	deactivate: 'stopStore',
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
	    handler: 'runSyncJob',
	    disabled: true,
	},
    ],

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
	    header: gettext('Remote'),
	    dataIndex: 'remote',
	    width: 120,
	    sortable: true,
	},
	{
	    header: gettext('Remote Store'),
	    dataIndex: 'remote-store',
	    width: 120,
	    sortable: true,
	},
	{
	    header: gettext('Local Store'),
	    dataIndex: 'store',
	    width: 120,
	    sortable: true,
	},
	{
	    header: gettext('Owner'),
	    dataIndex: 'owner',
	    renderer: 'render_optional_owner',
	    flex: 2,
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
	    header: gettext('Last Sync'),
	    dataIndex: 'last-run-endtime',
	    renderer: 'render_optional_timestamp',
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
	    renderer: 'render_sync_status',
	    flex: 3,
	},
	{
	    header: gettext('Next Run'),
	    dataIndex: 'next-run',
	    renderer: 'render_next_run',
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
});
