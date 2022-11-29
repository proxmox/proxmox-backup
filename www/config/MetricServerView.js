Ext.define('PBS.config.MetricServerView', {
    extend: 'Ext.grid.Panel',
    alias: ['widget.pbsMetricServerView'],

    stateful: true,
    stateId: 'grid-metricserver',

    controller: {
	xclass: 'Ext.app.ViewController',

	editWindow: function(xtype, id) {
	    let me = this;
	    Ext.create(`PBS.window.${xtype}Edit`, {
		serverid: id,
		autoShow: true,
		autoLoad: !!id,
		listeners: {
		    destroy: () => me.reload(),
		},
	    });
	},

	addServer: function(button) {
	    this.editWindow(PBS.Schema.metricServer[button.type]?.xtype);
	},

	editServer: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (!selection || selection.length < 1) {
		return;
	    }

	    let cfg = selection[0].data;

	    me.editWindow(PBS.Schema.metricServer[cfg.type]?.xtype, cfg.name);
	},

	reload: function() {
	    this.getView().getStore().load();
	},
    },

    store: {
	autoLoad: true,
	id: 'metricservers',
	proxy: {
	    type: 'proxmox',
	    url: '/api2/json/admin/metrics',
	},
    },

    columns: [
	{
	    text: gettext('Name'),
	    flex: 2,
	    dataIndex: 'name',
	},
	{
	    text: gettext('Type'),
	    width: 150,
	    dataIndex: 'type',
	    renderer: (v) => PBS.Schema.metricServer[v]?.type ?? v,
	},
	{
	    text: gettext('Enabled'),
	    dataIndex: 'enable',
	    width: 100,
	    renderer: v => Proxmox.Utils.format_boolean(v ?? true),
	},
	{
	    text: gettext('Target Server'),
	    width: 200,
	    dataIndex: 'server',
	},
	{
	    text: gettext('Comment'),
	    flex: 3,
	    dataIndex: 'comment',
	    renderer: Ext.htmlEncode,
	},
    ],

    tbar: [
	{
	    text: gettext('Add'),
	    menu: [
		{
		    text: 'InfluxDB (HTTP)',
		    type: 'influxdb-http',
		    iconCls: 'fa fa-fw fa-bar-chart',
		    handler: 'addServer',
		},
		{
		    text: 'InfluxDB (UDP)',
		    type: 'influxdb-udp',
		    iconCls: 'fa fa-fw fa-bar-chart',
		    handler: 'addServer',
		},
	    ],
	},
	{
	    text: gettext('Edit'),
	    xtype: 'proxmoxButton',
	    handler: 'editServer',
	    disabled: true,
	},
	{
	    xtype: 'proxmoxStdRemoveButton',
	    getUrl: (rec) => `/api2/extjs/config/metrics/${rec.data.type}/${rec.data.name}`,
	    getRecordName: (rec) => rec.data.name,
	    callback: 'reload',
	},
    ],

    listeners: {
	itemdblclick: 'editServer',
    },

    initComponent: function() {
	var me = this;

	me.callParent();

	Proxmox.Utils.monStoreErrors(me, me.getStore());
    },
});
