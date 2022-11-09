Ext.define('pbs-datastore-statistics', {
    extend: 'Ext.data.Model',

    fields: [
	'store',
	{
	    name: 'total',
	    calculate: function(data) {
		return data.avail + data.used;
	    },
	},
	'used',
	'avail',
	'estimated-full-date',
	'error',
	{
	    name: 'history',
	    convert: function(values) {
		if (!values) {
		    return [];
		}
		let last = null;
		return values.map(v => {
		    if (v !== undefined && v !== null) {
			last = v;
		    }
		    return last;
		});
	    },
	},
	{
	    name: 'usage',
	    calculate: function(data) {
		let used = data.used || 0;
		let total = data.total || 0;
		if (total > 0) {
		    return used/total;
		} else {
		    return -1;
		}
	    },
	},
    ],

    proxy: {
        type: 'proxmox',
	url: "/api2/json/status/datastore-usage",
    },
    idProperty: 'store',
});

Ext.define('PBS.DatastoreStatistics', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.pbsDatastoresStatistics',

    title: gettext('Datastore Usage'),
    disableSelection: true,

    emptyText: gettext('No Data'),

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    Proxmox.Utils.monStoreErrors(view, view.getStore().rstore);
	},
    },

    columns: [
	{
	    text: gettext('Name'),
	    dataIndex: 'store',
	    sortable: true,
	    renderer: (value, metaData, record, rowIndex, colIndex, store) => {
		let err = record.get('error');
		if (err) {
		    metaData.tdAttr = `data-qtip="${Ext.htmlEncode(err)}"`;
		    metaData.tdCls = 'proxmox-invalid-row';
		    return `${value || ''} <i class="fa fa-fw critical fa-exclamation-circle"></i>`;
		}
		return value;
	    },
	},
	{
	    text: gettext('Size'),
	    dataIndex: 'total',
	    sortable: true,
	    width: 90,
	    renderer: v => v === undefined || v < 0 ? '-' : Proxmox.Utils.format_size(v, true),
	},
	{
	    text: gettext('Used'),
	    dataIndex: 'used',
	    sortable: true,
	    width: 90,
	    renderer: v => v === undefined || v < 0 ? '-' : Proxmox.Utils.format_size(v, true),
	},
	{
	    text: gettext('Available'),
	    dataIndex: 'avail',
	    sortable: true,
	    width: 90,
	    renderer: v => v === undefined || v < 0 ? '-' : Proxmox.Utils.format_size(v, true),
	},
	{
	    text: gettext('Usage %'),
	    dataIndex: 'usage',
	    sortable: true,
	    xtype: 'widgetcolumn',
	    widget: {
		xtype: 'progressbarwidget',
		bind: '{record.usage}',
		textTpl: [
		    '<tpl if="value &gt;= 0">',
		    '{value:number("0.00")*100}%',
		    '<tpl else>',
		    Proxmox.Utils.unknownText,
		    '</tpl>',
		],
	    },
	},
	{
	    text: gettext('Estimated Full'),
	    dataIndex: 'estimated-full-date',
	    sortable: true,
	    renderer: PBS.Utils.render_estimate,
	    flex: 1,
	    minWidth: 130,
	    maxWidth: 200,
	},
	{
	    text: gettext("History (last Month)"),
	    width: 100,
	    xtype: 'widgetcolumn',
	    dataIndex: 'history',
	    flex: 1,
	    widget: {
		xtype: 'sparklineline',
		bind: '{record.history}',
		spotRadius: 0,
		fillColor: '#ddd',
		lineColor: '#555',
		lineWidth: 0,
		chartRangeMin: 0,
		chartRangeMax: 1,
		tipTpl: '{y:number("0.00")*100}%',
	    },
	},
    ],

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: 'store',
	rstore: {
	    type: 'update',
	    storeid: 'pbs-datastore-statistics',
	    model: 'pbs-datastore-statistics',
	    autoStart: true,
	    interval: 30000,
	},
    },

});
