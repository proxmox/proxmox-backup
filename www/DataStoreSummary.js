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

Ext.define('PBS.DataStoreInfo', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsDataStoreInfo',

    viewModel: {
	data: {
	    countstext: '',
	    usage: {},
	    stillbad: 0,
	    removedbytes: 0,
	    mountpoint: "",
	},
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	onLoad: function(store, data, success) {
	    if (!success) return;
	    let me = this;
	    let vm = me.getViewModel();

	    let counts = store.getById('counts').data.value;
	    let storage = store.getById('storage').data.value;

	    let used = Proxmox.Utils.format_size(storage.used);
	    let total = Proxmox.Utils.format_size(storage.total);
	    let percent = 100*storage.used/storage.total;
	    if (storage.total === 0) {
		percent = 0;
	    }
	    let used_percent = `${percent.toFixed(2)}%`;

	    let usage = used_percent + ' (' +
		Ext.String.format(gettext('{0} of {1}'),
				  used, total) + ')';
	    vm.set('usagetext', usage);
	    vm.set('usage', storage.used/storage.total);

	    let gcstatus = store.getById('gc-status').data.value;

	    let dedup = (gcstatus['index-data-bytes'] || 0)/
			(gcstatus['disk-bytes'] || Infinity);

	    let countstext = function(count) {
		return `${count[0]} ${gettext('Groups')}, ${count[1]} ${gettext('Snapshots')}`;
	    };

	    vm.set('ctcount', countstext(counts.ct || [0, 0]));
	    vm.set('vmcount', countstext(counts.vm || [0, 0]));
	    vm.set('hostcount', countstext(counts.host || [0, 0]));
	    vm.set('deduplication', dedup.toFixed(2));
	    vm.set('stillbad', gcstatus['still-bad']);
	    vm.set('removedbytes', Proxmox.Utils.format_size(gcstatus['removed-bytes']));
	},

	startStore: function() { this.store.startUpdate(); },
	stopStore: function() { this.store.stopUpdate(); },

	init: function(view) {
	    let me = this;
	    let datastore = encodeURIComponent(view.datastore);
	    me.store = Ext.create('Proxmox.data.ObjectStore', {
		interval: 5*1000,
		url: `/api2/json/admin/datastore/${datastore}/status`,
	    });
	    me.store.on('load', me.onLoad, me);
	},
    },

    listeners: {
	activate: 'startStore',
	destroy: 'stopStore',
	deactivate: 'stopStore',
    },

    defaults: {
	xtype: 'pmxInfoWidget',
    },

    bodyPadding: 20,

    items: [
	{
	    iconCls: 'fa fa-hdd-o',
	    title: gettext('Usage'),
	    bind: {
		data: {
		    usage: '{usage}',
		    text: '{usagetext}',
		},
	    },
	},
	{
	    xtype: 'box',
	    html: `<b>${gettext('Backup Count')}</b>`,
	    padding: '10 0 5 0',
	},
	{
	    iconCls: 'fa fa-cube',
	    title: gettext('CT'),
	    printBar: false,
	    bind: {
		data: {
		    text: '{ctcount}',
		},
	    },
	},
	{
	    iconCls: 'fa fa-building',
	    title: gettext('Host'),
	    printBar: false,
	    bind: {
		data: {
		    text: '{hostcount}',
		},
	    },
	},
	{
	    iconCls: 'fa fa-desktop',
	    title: gettext('VM'),
	    printBar: false,
	    bind: {
		data: {
		    text: '{vmcount}',
		},
	    },
	},
	{
	    xtype: 'box',
	    html: `<b>${gettext('Stats from last Garbage Collection')}</b>`,
	    padding: '10 0 5 0',
	},
	{
	    iconCls: 'fa fa-compress',
	    title: gettext('Deduplication'),
	    printBar: false,
	    bind: {
		data: {
		    text: '{deduplication}',
		},
	    },
	},
	{
	    iconCls: 'fa fa-trash-o',
	    title: gettext('Removed Bytes'),
	    printBar: false,
	    bind: {
		data: {
		    text: '{removedbytes}',
		},
	    },
	},
	{
	    iconCls: 'fa critical fa-exclamation-triangle',
	    title: gettext('Bad Chunks'),
	    printBar: false,
	    bind: {
		data: {
		    text: '{stillbad}',
		},
		visible: '{stillbad}',
	    },
	},
    ],
});

Ext.define('PBS.DataStoreSummary', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsDataStoreSummary',
    mixins: ['Proxmox.Mixin.CBind'],

    layout: 'column',
    scrollable: true,

    bodyPadding: 5,
    defaults: {
	columnWidth: 1,
	padding: 5,
    },

    tbar: ['->', { xtype: 'proxmoxRRDTypeSelector' }],

    items: [
	{
	    xtype: 'container',
	    height: 300,
	    layout: {
		type: 'hbox',
		align: 'stretch',
	    },
	    items: [
		{
		    xtype: 'pbsDataStoreInfo',
		    flex: 1,
		    padding: '0 10 0 0',
		    cbind: {
			title: '{datastore}',
			datastore: '{datastore}',
		    },
		},
		{
		    xtype: 'pbsDataStoreNotes',
		    flex: 1,
		    cbind: {
			datastore: '{datastore}',
		    },
		},
	    ],
	},
	{
	    xtype: 'proxmoxRRDChart',
	    title: gettext('Storage usage (bytes)'),
	    fields: ['total', 'used'],
	    fieldTitles: [gettext('Total'), gettext('Storage usage')],
	},
	{
	    xtype: 'proxmoxRRDChart',
	    title: gettext('Transfer Rate (bytes/second)'),
	    fields: ['read_bytes', 'write_bytes'],
	    fieldTitles: [gettext('Read'), gettext('Write')],
	},
	{
	    xtype: 'proxmoxRRDChart',
	    title: gettext('Input/Output Operations per Second (IOPS)'),
	    fields: ['read_ios', 'write_ios'],
	    fieldTitles: [gettext('Read'), gettext('Write')],
	},
	{
	    xtype: 'proxmoxRRDChart',
	    title: gettext('IO Delay (ms)'),
	    fields: ['io_delay'],
	    fieldTitles: [gettext('IO Delay')],
	},
    ],

    listeners: {
	activate: function() { this.rrdstore.startUpdate(); },
	deactivate: function() { this.rrdstore.stopUpdate(); },
	destroy: function() { this.rrdstore.stopUpdate(); },
    },

    initComponent: function() {
	let me = this;

	me.rrdstore = Ext.create('Proxmox.data.RRDStore', {
	    rrdurl: "/api2/json/admin/datastore/" + me.datastore + "/rrd",
	    model: 'pve-rrd-datastore',
	});

	me.callParent();

	Proxmox.Utils.API2Request({
	    url: `/config/datastore/${me.datastore}`,
	    waitMsgTarget: me.down('pbsDataStoreInfo'),
	    success: function(response) {
		let path = Ext.htmlEncode(response.result.data.path);
		me.down('pbsDataStoreInfo').setTitle(`${me.datastore} (${path})`);
		me.down('pbsDataStoreNotes').setNotes(response.result.data.comment);
	    },
	});

	me.query('proxmoxRRDChart').forEach((chart) => {
	    chart.setStore(me.rrdstore);
	});

	me.down('pbsDataStoreInfo').relayEvents(me, ['activate', 'deactivate']);
    },
});
