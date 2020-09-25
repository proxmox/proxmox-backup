Ext.define('pve-rrd-datastore', {
    extend: 'Ext.data.Model',
    fields: [
	'used',
	'total',
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
		if (data.io_ticks === undefined) {
		    return undefined;
		} else if (ios === 0) {
		    return 0;
		}
		return (data.io_ticks*1000.0)/ios;
	    },
	},
	{ type: 'date', dateFormat: 'timestamp', name: 'time' },
    ],
});

Ext.define('PBS.DataStoreStatistic', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsDataStoreStatistic',

    title: gettext('Statistics'),

    scrollable: true,

    initComponent: function() {
        var me = this;

	if (!me.datastore) {
	    throw "no datastore specified";
	}

	me.tbar = ['->', { xtype: 'proxmoxRRDTypeSelector' }];

	var rrdstore = Ext.create('Proxmox.data.RRDStore', {
	    rrdurl: "/api2/json/admin/datastore/" + me.datastore + "/rrd",
	    model: 'pve-rrd-datastore',
	});

	me.items = {
	    xtype: 'container',
	    itemId: 'itemcontainer',
	    layout: 'column',
	    minWidth: 700,
	    defaults: {
		minHeight: 320,
		padding: 5,
		columnWidth: 1,
	    },
	    items: [
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('Storage usage (bytes)'),
		    fields: ['total', 'used'],
		    fieldTitles: [gettext('Total'), gettext('Storage usage')],
		    store: rrdstore,
		},
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('Transfer Rate (bytes/second)'),
		    fields: ['read_bytes', 'write_bytes'],
		    fieldTitles: [gettext('Read'), gettext('Write')],
		    store: rrdstore,
		},
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('Input/Output Operations per Second (IOPS)'),
		    fields: ['read_ios', 'write_ios'],
		    fieldTitles: [gettext('Read'), gettext('Write')],
		    store: rrdstore,
		},
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('IO Delay (ms)'),
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
    },

});
