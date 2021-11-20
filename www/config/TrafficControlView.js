Ext.define('pmx-traffic-control', {
    extend: 'Ext.data.Model',
    fields: [
	'name', 'rate-in', 'rate-out', 'burst-in', 'burst-out', 'network',
	'timeframe', 'comment', 'cur-rate-in', 'cur-rate-out',
	{
	    name: 'rateInUsed',
	    calculate: d => Proxmox.Utils.size_unit_ratios(d['cur-rate-in'], d['rate-in']),
	},
	{
	    name: 'rateOutUsed',
	    calculate: d => Proxmox.Utils.size_unit_ratios(d['cur-rate-out'], d['rate-out']),
	},
    ],
    idProperty: 'name',
    proxy: {
	type: 'proxmox',
	url: '/api2/json/admin/traffic-control',
    },
});

Ext.define('PBS.config.TrafficControlView', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsTrafficControlView',

    stateful: true,
    stateId: 'grid-traffic-control',

    title: gettext('Traffic Control'),

//    tools: [PBS.Utils.get_help_tool("backup-remote")], // TODO!

    controller: {
	xclass: 'Ext.app.ViewController',

	addRemote: function() {
	    let me = this;
            Ext.create('PBS.window.TrafficControlEdit', {
		autoShow: true,
		listeners: {
		    destroy: () => me.reload(),
		},
            });
	},

	editTrafficControl: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (selection.length < 1) return;

	    Ext.create('PBS.window.TrafficControlEdit', {
		name: selection[0].data.name,
		autoShow: true,
		listeners: {
		    destroy: () => me.reload(),
		},
            });
	},

	render_bandwidth: v => v ? Proxmox.Utils.autoscale_size_unit(v) + '/s' : '',

	reload: function() { this.getView().getStore().rstore.load(); },

	init: function(view) {
	    Proxmox.Utils.monStoreErrors(view, view.getStore().rstore);
	},
    },

    listeners: {
	activate: 'reload',
	itemdblclick: 'editTrafficControl',
    },

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: 'name',
	rstore: {
	    type: 'update',
	    storeid: 'pmx-traffic-control',
	    model: 'pmx-traffic-control',
	    autoStart: true,
	    interval: 5000,
	},
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Add'),
	    handler: 'addRemote',
	    selModel: false,
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Edit'),
	    handler: 'editTrafficControl',
	    disabled: true,
	},
	{
	    xtype: 'proxmoxStdRemoveButton',
	    baseurl: '/config/traffic-control',
	    callback: 'reload',
	},
    ],

    viewConfig: {
	trackOver: false,
    },

    columns: [
	{
	    header: gettext('Rule'),
	    width: 120,
	    sortable: true,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'name',
	},
	{
	    header: gettext('Rate In'),
	    width: 120,
	    sortable: true,
	    renderer: 'render_bandwidth',
	    dataIndex: 'rate-in',
	},
	{
	    header: gettext('Rate In Used'),
	    xtype: 'widgetcolumn',
	    dataIndex: 'rateInUsed',
	    //width: 200,
	    flex: 2,
	    widget: {
		xtype: 'progressbarwidget',
		textTpl: '{percent:number("0")}%',
		animate: true,
	    },
	},
	{
	    header: gettext('Rate Out'),
	    width: 120,
	    sortable: true,
	    renderer: 'render_bandwidth',
	    dataIndex: 'rate-out',
	},
	{
	    header: gettext('Rate Out Used'),
	    xtype: 'widgetcolumn',
	    dataIndex: 'rateOutUsed',
	    flex: 2,
	    widget: {
		xtype: 'progressbarwidget',
		textTpl: '{percent:number("0")}%',
		animate: true,
	    },
	},
	{
	    header: gettext('Burst In'),
	    width: 120,
	    sortable: true,
	    renderer: 'render_bandwidth',
	    dataIndex: 'burst-in',
	},
	{
	    header: gettext('Burst Out'),
	    width: 120,
	    sortable: true,
	    renderer: 'render_bandwidth',
	    dataIndex: 'burst-out',
	},
	{
	    header: gettext('Networks'),
	    flex: 3,
	    sortable: true,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'network',
	},
	{
	    header: gettext('Timeframes'),
	    sortable: false,
	    renderer: tf => tf ? Ext.String.htmlEncode(tf.join('; ')) : '',
	    dataIndex: 'timeframe',
	    flex: 3,
	},
	{
	    header: gettext('Comment'),
	    sortable: false,
	    renderer: Ext.String.htmlEncode,
	    dataIndex: 'comment',
	    flex: 2,
	},
    ],
});
