Ext.define('pbs-datastore-list', {
    extend: 'Ext.data.Model',
    fields: [ 'name', 'comment' ],
    proxy: {
        type: 'proxmox',
	url: "/api2/json/admin/datastore"
    },
    idProperty: 'store'
});

Ext.define('pve-rrd-node', {
    extend: 'Ext.data.Model',
    fields: [
	{
	    name: 'cpu',
	    // percentage
	    convert: function(value) {
		return value*100;
	    }
	},
	{
	    name: 'iowait',
	    // percentage
	    convert: function(value) {
		return value*100;
	    }
	},
	"memtotal",
	"memused",
	{ type: 'date', dateFormat: 'timestamp', name: 'time' }
    ]
});


Ext.define('PBS.DataStoreStatus', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsDataStoreStatus',

    title: gettext('Data Store Status'),
    tbar: ['->', { xtype: 'proxmoxRRDTypeSelector' } ],

    initComponent: function() {
        var me = this;

	// this is just a test for the RRD api

	var rrdstore = Ext.create('Proxmox.data.RRDStore', {
	    rrdurl: "/api2/json/nodes/localhost/rrd",
	    model: 'pve-rrd-node'
	});

	me.items = {
	    xtype: 'container',
	    itemId: 'itemcontainer',
	    layout: 'column',
	    minWidth: 700,
	    defaults: {
		minHeight: 320,
		padding: 5,
		columnWidth: 1
	    },
	    items: [
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('CPU usage'),
		    fields: ['cpu','iowait'],
		    fieldTitles: [gettext('CPU usage'), gettext('IO delay')],
		    store: rrdstore
		},
		{
		    xtype: 'proxmoxRRDChart',
		    title: gettext('Memory usage'),
		    fields: ['memtotal','memused'],
		    fieldTitles: [gettext('Total'), gettext('RAM usage')],
		    store: rrdstore
		},
	    ]
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
    }
});
