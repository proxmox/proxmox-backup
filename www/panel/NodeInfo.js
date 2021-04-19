Ext.define('PBS.NodeInfoPanel', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsNodeInfoPanel',

    iconCls: 'fa fa-tasks',
    title: gettext('Server Resources'),

    controller: {
	xclass: 'Ext.app.ViewController',

	showFingerPrint: function() {
	    let me = this;
	    let view = me.getView();
	    let fingerprint = view.fingerprint;
	    Ext.create('Ext.window.Window', {
		modal: true,
		width: 600,
		title: gettext('Fingerprint'),
		layout: 'form',
		bodyPadding: '10 0',
		items: [
		    {
			xtype: 'textfield',
			inputId: 'fingerprintField',
			value: fingerprint,
			editable: false,
		    },
		],
		buttons: [
		    {
			xtype: 'button',
			iconCls: 'fa fa-clipboard',
			handler: function(b) {
			    var el = document.getElementById('fingerprintField');
			    el.select();
			    document.execCommand("copy");
			},
			text: gettext('Copy'),
		    },
		    {
			text: gettext('Ok'),
			handler: function() {
			    this.up('window').close();
			},
		    },
		],
	    }).show();
	},

	updateUsageStats: function(store, records, success) {
	    if (!success) {
		return;
	    }
	    if (records === undefined || records.length < 1) {
		return;
	    }
	    let me = this;
	    let view = me.getView();

	    let res = records[0].data;
	    view.fingerprint = res.info.fingerprint;

	    me.lookup('fpButton').setDisabled(!view.fingerprint);

	    let cpu = res.cpu,
		mem = res.memory,
		root = res.root;

	    var cpuPanel = me.lookup('cpu');
	    cpuPanel.updateValue(cpu);

	    var memPanel = me.lookup('mem');
	    memPanel.updateValue(mem.used / mem.total);

	    var hdPanel = me.lookup('root');
	    hdPanel.updateValue(root.used / root.total);
	},

	init: function(view) {
	    let me = this;

	    view.store = Ext.create('Proxmox.data.UpdateStore', {
		interval: 3000,
		proxy: {
		    type: 'proxmox',
		    url: '/api2/json/nodes/localhost/status',
		},
	    });

	    me.mon(view.store, 'load', me.updateUsageStats, me);

	    view.store.startUpdate();
	},

	startStore: function() {
	    let me = this;
	    let view = me.getView();
	    view.store.startUpdate();
	},

	stopStore: function() {
	    let me = this;
	    let view = me.getView();
	    view.store.stopUpdate();
	},
    },

    listeners: {
	activate: 'startStore',
	deactivate: 'stopStore',
	destroy: 'stopStore',
    },

    bodyPadding: '0 20 0 20',

    tools: [
	{
	    xtype: 'button',
	    reference: 'fpButton',
	    text: gettext('Show Fingerprint'),
	    handler: 'showFingerPrint',
	    disabled: true,
	},
    ],

    layout: {
	type: 'hbox',
	align: 'center',
    },

    defaults: {
	xtype: 'proxmoxGauge',
	spriteFontSize: '20px',
	flex: 1,
    },

    items: [
	{
	    title: gettext('CPU'),
	    reference: 'cpu',
	},
	{
	    title: gettext('Memory'),
	    reference: 'mem',
	},
	{
	    title: gettext('Root Disk'),
	    reference: 'root',
	},
    ],
});
