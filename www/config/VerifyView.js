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
    stateId: 'grid-verify-jobs',

    title: gettext('Verify Jobs'),

    controller: {
	xclass: 'Ext.app.ViewController',

	addVerifyJob: function() {
	    let me = this;
	    Ext.create('PBS.window.VerifyJobEdit', {
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

	render_verify_status: function(value, metadata, record) {
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

	reload: function() { this.getView().getStore().rstore.load(); },

	init: function(view) {
	    Proxmox.Utils.monStoreErrors(view, view.getStore().rstore);
	},
    },

    listeners: {
	activate: 'reload',
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
	    autoStart: true,
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
	    handler: 'runVerifyJob',
	    disabled: true,
	},
    ],

    viewConfig: {
	trackOver: false,
    },

    columns: [
	{
	    header: gettext('Verify Job'),
	    width: 100,
	    sortable: true,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'id',
	},
	{
	    header: gettext('Datastore'),
	    width: 100,
	    sortable: true,
	    dataIndex: 'store',
	},
	{
	    header: gettext('Days valid'),
	    width: 125,
	    sortable: true,
	    dataIndex: 'outdated-after',
	},
	{
	    header: gettext('Ignore verified'),
	    width: 125,
	    sortable: true,
	    renderer: Proxmox.Utils.format_boolean,
	    dataIndex: 'ignore-verified',
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
	    renderer: 'render_verify_status',
	},
	{
	    header: gettext('Last Verification'),
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
