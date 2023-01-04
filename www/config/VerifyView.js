Ext.define('pbs-verify-jobs-status', {
    extend: 'Ext.data.Model',
    fields: [
	'id', 'store', 'outdated-after', 'ignore-verified', 'schedule',
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
	'ns',
	'max-depth',
    ],
    idProperty: 'id',
    proxy: {
	type: 'proxmox',
	url: '/api2/json/admin/verify',
    },
});

Ext.define('PBS.config.VerifyJobView', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsVerifyJobView',

    stateful: true,
    stateId: 'grid-verify-jobs-v1',

    title: gettext('Verify Jobs'),

    controller: {
	xclass: 'Ext.app.ViewController',

	addVerifyJob: function() {
	    let me = this;
	    let view = me.getView();
	    Ext.create('PBS.window.VerifyJobEdit', {
		datastore: view.datastore,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	editVerifyJob: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

	    Ext.create('PBS.window.VerifyJobEdit', {
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

	runVerifyJob: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

	    let id = selection[0].data.id;
	    Proxmox.Utils.API2Request({
		method: 'POST',
		url: `/admin/verify/${id}/run`,
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
	itemdblclick: 'editVerifyJob',
    },

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: 'id',
	rstore: {
	    type: 'update',
	    storeid: 'pbs-verify-jobs-status',
	    model: 'pbs-verify-jobs-status',
	    interval: 5000,
	},
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Add'),
	    handler: 'addVerifyJob',
	    selModel: false,
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Edit'),
	    handler: 'editVerifyJob',
	    disabled: true,
	},
	{
	    xtype: 'proxmoxStdRemoveButton',
	    baseurl: '/config/verify/',
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
	    handler: 'runVerifyJob',
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
	    header: gettext('Datastore'),
	    dataIndex: 'store',
	    minWidth: 80,
	    flex: 1,
	},
	{
	    header: gettext('Namespace'),
	    dataIndex: 'ns',
	    minWidth: 80,
	    flex: 2,
	    sortable: true,
	    renderer: PBS.Utils.render_optional_namespace,
	},
	{
	    header: gettext('Max. Depth'),
	    dataIndex: 'max-depth',
	    width: 90,
	    sortable: true,
	},
	{
	    header: gettext('Skip Verified'),
	    dataIndex: 'ignore-verified',
	    renderer: Proxmox.Utils.format_boolean,
	    width: 100,
	    sortable: true,
	},
	{
	    header: gettext('Re-Verify After'),
	    dataIndex: 'outdated-after',
	    renderer: v => v ? v +' '+ gettext('Days') : gettext('Never'),
	    width: 125,
	    sortable: true,
	},
	{
	    header: gettext('Schedule'),
	    dataIndex: 'schedule',
	    sortable: true,
	    maxWidth: 220,
	    minWidth: 80,
	    flex: 1,
	},
	{
	    header: gettext('Last Verification'),
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
	let hideDatastore = !!me.datastore;

	for (let column of me.columns) {
	    if (column.dataIndex === 'store') {
		column.hidden = hideDatastore;
		break;
	    }
	}

	me.callParent();
    },
});
