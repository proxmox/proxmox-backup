Ext.define('pve-rrd-node', {
    extend: 'Ext.data.Model',
    fields: [
	{
	    name: 'cpu',
	    // percentage
	    convert: function(value) {
		return value*100;
	    },
	},
	{
	    name: 'iowait',
	    // percentage
	    convert: function(value) {
		return value*100;
	    },
	},
	'netin',
	'netout',
	'memtotal',
	'memused',
	'swaptotal',
	'swapused',
	'total',
	'used',
	'read_ios',
	'read_bytes',
        'write_ios',
	'write_bytes',
        'io_ticks',
	{
	    name: 'io_delay', calculate: function(data) {
		let ios = 0;
		if (data.read_ios !== undefined) { ios += data.read_ios; }
		if (data.write_ios !== undefined) { ios += data.write_ios; }
		if (ios === 0 || data.io_ticks === undefined) {
		    return undefined;
		}
		return (data.io_ticks*1000.0)/ios;
	    },
	},
	'loadavg',
	{ type: 'date', dateFormat: 'timestamp', name: 'time' },
    ],
});
Ext.define('PBS.ServerStatus', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsServerStatus',

    title: gettext('Server Status'),

    scrollable: true,

    showVersions: function() {
	let me = this;

	// Note: use simply text/html here, as ExtJS grid has problems with cut&paste
	let panel = Ext.createWidget('component', {
	    autoScroll: true,
	    id: 'pkgversions',
	    padding: 5,
	    style: {
		'white-space': 'pre',
		'font-family': 'monospace',
	    },
	});

	let win = Ext.create('Ext.window.Window', {
	    title: gettext('Package versions'),
	    width: 600,
	    height: 600,
	    layout: 'fit',
	    modal: true,
	    items: [panel],
	    buttons: [
		{
		    xtype: 'button',
		    iconCls: 'fa fa-clipboard',
		    handler: function(button) {
			window.getSelection().selectAllChildren(
			    document.getElementById('pkgversions'),
			);
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
	});

	Proxmox.Utils.API2Request({
	    waitMsgTarget: me,
	    url: `/nodes/localhost/apt/versions`,
	    method: 'GET',
	    failure: function(response, opts) {
		win.close();
		Ext.Msg.alert(gettext('Error'), response.htmlStatus);
	    },
	    success: function(response, opts) {
		let text = '';
		Ext.Array.each(response.result.data, function(rec) {
		    let version = "not correctly installed";
		    let pkg = rec.Package;
		    if (rec.OldVersion && rec.OldVersion !== 'unknown') {
			version = rec.OldVersion;
		    }
		    if (rec.ExtraInfo) {
			text += `${pkg}: ${version} (${rec.ExtraInfo})\n`;
		    } else {
			text += `${pkg}: ${version}\n`;
		    }
		});

		win.show();
		panel.update(Ext.htmlEncode(text));
	    },
	});
    },

    initComponent: function() {
        var me = this;

	var node_command = function(cmd) {
	    Proxmox.Utils.API2Request({
		params: { command: cmd },
		url: '/nodes/localhost/status',
		method: 'POST',
		waitMsgTarget: me,
		failure: function(response, opts) {
		    Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		},
	    });
	};

	var restartBtn = Ext.create('Proxmox.button.Button', {
	    text: gettext('Reboot'),
	    dangerous: true,
	    confirmMsg: gettext("Reboot backup server?"),
	    handler: function() {
		node_command('reboot');
	    },
	    iconCls: 'fa fa-undo',
	});

	var shutdownBtn = Ext.create('Proxmox.button.Button', {
	    text: gettext('Shutdown'),
	    dangerous: true,
	    confirmMsg: gettext("Shutdown backup server?"),
	    handler: function() {
		node_command('shutdown');
	    },
	    iconCls: 'fa fa-power-off',
	});

	var consoleBtn = Ext.create('Proxmox.button.Button', {
	    text: gettext('Console'),
	    iconCls: 'fa fa-terminal',
	    handler: function() {
		Proxmox.Utils.openXtermJsViewer('shell', 0, Proxmox.NodeName);
	    },
	});

	let version_btn = new Ext.Button({
	    text: gettext('Package versions'),
	    iconCls: 'fa fa-gift',
	    handler: function() {
		Proxmox.Utils.checked_command(function() { me.showVersions(); });
	    },
	});

	me.tbar = [version_btn, '-', consoleBtn, '-', restartBtn, shutdownBtn, '->', { xtype: 'proxmoxRRDTypeSelector' }];

	var rrdstore = Ext.create('Proxmox.data.RRDStore', {
	    rrdurl: "/api2/json/nodes/localhost/rrd",
	    model: 'pve-rrd-node',
	});

	me.items = {
	    xtype: 'container',
	    itemId: 'itemcontainer',
	    layout: 'column',
	    minWidth: 700,
	    listeners: {
		resize: function(panel) {
		    Proxmox.Utils.updateColumns(panel);
		},
	    },
	    defaults: {
		minHeight: 320,
		padding: 5,
		columnWidth: 1,
	    },
	    items: [
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('CPU usage'),
		    fields: ['cpu', 'iowait'],
		    fieldTitles: [gettext('CPU usage'), gettext('IO wait')],
		    store: rrdstore,
		},
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('Server load'),
		    fields: ['loadavg'],
		    fieldTitles: [gettext('Load average')],
		    store: rrdstore,
		},
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('Memory usage'),
		    fields: ['memtotal', 'memused'],
		    fieldTitles: [gettext('Total'), gettext('RAM usage')],
		    unit: 'bytes',
		    powerOfTwo: true,
		    store: rrdstore,
		},
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('Swap usage'),
		    fields: ['swaptotal', 'swapused'],
		    fieldTitles: [gettext('Total'), gettext('Swap usage')],
		    unit: 'bytes',
		    powerOfTwo: true,
		    store: rrdstore,
		},
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('Network traffic'),
		    fields: ['netin', 'netout'],
		    store: rrdstore,
		},
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('Root Disk usage'),
		    fields: ['total', 'used'],
		    fieldTitles: [gettext('Total'), gettext('Disk usage')],
		    store: rrdstore,
		},
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('Root Disk Transfer Rate (bytes/second)'),
		    fields: ['read_bytes', 'write_bytes'],
		    fieldTitles: [gettext('Read'), gettext('Write')],
		    store: rrdstore,
		},
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('Root Disk Input/Output Operations per Second (IOPS)'),
		    fields: ['read_ios', 'write_ios'],
		    fieldTitles: [gettext('Read'), gettext('Write')],
		    store: rrdstore,
		},
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('Root Disk IO Delay (ms)'),
		    fields: ['io_delay'],
		    fieldTitles: [gettext('IO Delay')],
		    store: rrdstore,
		},
	    ],
	};

	me.listeners = {
	    activate: function() {
		rrdstore.startUpdate();
	    },
	    destroy: function() {
		rrdstore.stopUpdate();
	    },
	};

	me.callParent();

	let sp = Ext.state.Manager.getProvider();
	me.mon(sp, 'statechange', function(provider, key, value) {
	    if (key !== 'summarycolumns') {
		return;
	    }
	    Proxmox.Utils.updateColumns(me.getComponent('itemcontainer'));
	});
    },

});
