Ext.define('pbs-prune-jobs-status', {
    extend: 'Ext.data.Model',
    fields: [
	'id', 'disable', 'store', 'ns', 'max-depth', 'schedule',
	'keep-last', 'keep-hourly', 'keep-daily', 'keep-weekly', 'keep-monthly', 'keep-yearly',
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
	url: '/api2/json/admin/prune',
    },
});

Ext.define('PBS.config.PruneJobView', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsPruneJobView',

    stateful: true,
    stateId: 'grid-prune-jobs-v1',

    title: gettext('Prune Jobs'),

    controller: {
	xclass: 'Ext.app.ViewController',

	addPruneJob: function() {
	    let me = this;
	    let view = me.getView();
            Ext.create('PBS.window.PruneJobEdit', {
		datastore: view.datastore,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
            }).show();
	},

	editPruneJob: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

            Ext.create('PBS.window.PruneJobEdit', {
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

	runPruneJob: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

	    let id = selection[0].data.id;
	    Proxmox.Utils.API2Request({
		method: 'POST',
		url: `/admin/prune/${id}/run`,
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

	render_optional_owner: function(value, metadata, record) {
	    if (!value) return '-';
	    return Ext.String.htmlEncode(value);
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
	itemdblclick: 'editPruneJob',
    },

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: 'id',
	rstore: {
	    type: 'update',
	    storeid: 'pbs-prune-jobs-status',
	    model: 'pbs-prune-jobs-status',
	    interval: 5000,
	},
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Add'),
	    handler: 'addPruneJob',
	    selModel: false,
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Edit'),
	    handler: 'editPruneJob',
	    disabled: true,
	},
	{
	    xtype: 'proxmoxStdRemoveButton',
	    baseurl: '/config/prune/',
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
	    handler: 'runPruneJob',
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
	    header: gettext('Store'),
	    dataIndex: 'store',
	    width: 120,
	    sortable: true,
	},
	{
	    header: gettext('Namespace'),
	    dataIndex: 'ns',
	    width: 120,
	    sortable: true,
	    renderer: PBS.Utils.render_optional_namespace,
	},
	{
	    header: gettext('Max. Recursion'),
	    dataIndex: 'max-depth',
	    width: 40,
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
	    text: gettext('Keep'),
	    defaults: {
		width: 60,
	    },
	    columns: [
		['last', gettext('Last')],
		['hourly', gettext('Hourly')],
		['daily', gettext('Daily')],
		['weekly', gettext('Weekly')],
		['monthly', gettext('Monthly')],
		['yearly', gettext('Yearly')],
	    ].map(([data, header]) => ({
		header: header,
		dataIndex: `keep-${data}`,
	    })),
	},
	{
	    header: gettext('Last Prune'),
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
	let hideLocalDatastore = !!me.datastore;

	for (let column of me.columns) {
	    if (column.dataIndex === 'store') {
		column.hidden = hideLocalDatastore;
		break;
	    }
	}

	me.callParent();
    },
});
