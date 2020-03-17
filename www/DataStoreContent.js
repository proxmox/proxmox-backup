Ext.define('pbs-data-store-content', {
    extend: 'Ext.data.Model',
    fields: [
	'backup-type',
	'backup-id',
	{
	    name: 'last-backup',
	    type: 'date',
	    dateFormat: 'timestamp'
	},
	'files',
	{ name: 'backup-count', type: 'int' },
	{
	    name: 'backup-group',
	    calculate: function (data) {
		return data["backup-type"] + '/' + data["backup-id"];
	    }
	},
    ],
});

Ext.define('PBS.DataStoreContent', {
    extend: 'Ext.grid.GridPanel',
    alias: 'widget.pbsDataStoreContent',

    store: {
	model: 'pbs-data-store-content',
	sorters: 'backup-group',
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    if (!view.datastore) {
		throw "no datastore specified";
	    }

	    view.title = gettext('Data Store Content: ') + view.datastore;

	    Proxmox.Utils.monStoreErrors(view, view.store, true);
	    this.reload(); // initial load
	},

	reload: function() {
	    var view = this.getView();

	    let url = `/api2/json/admin/datastore/${view.datastore}/groups`;
	    view.store.setProxy({
		type: 'proxmox',
		url:  url
	    });
	    view.store.load();
	},
    },

    initComponent: function() {
	var me = this;

	var render_backup_type = function(value, metaData, record) {
	    var btype = record.data["backup-type"];
	    var cls = '';
	    if (btype === 'vm') {
		cls = 'fa-desktop';
	    } else if (btype === 'ct') {
		cls = 'fa-cube';
	    } else if (btype === 'host') {
		cls = 'fa-building';
	    } else {
		return btype + '/' + value;
	    }
	    var fa = '<i class="fa fa-fw x-grid-icon-custom ' + cls  + '"></i> ';
	    return fa + value;
	};

	Ext.apply(me, {
	    columns: [
		{
		    header: gettext('Backup'),
		    sortable: true,
		    renderer: render_backup_type,
		    dataIndex: 'backup-id',
		    flex: 1
		},
		{
		    xtype: 'datecolumn',
		    header: gettext('Last Backup'),
		    sortable: true,
		    dataIndex: 'last-backup',
		    format: 'Y-m-d H:i:s',
		    flex: 1
		},
		{
		    xtype: 'numbercolumn',
		    format: '0',
		    header: gettext('Number of Backups'),
		    sortable: true,
		    dataIndex: 'backup-count',
		    flex: 1
		},
	    ],

	    plugins: [{
		ptype: 'rowexpander',
		rowBodyTpl: new Ext.XTemplate(
		    '<tpl for="files">',
		    '<p>{.}</p>',
		    '</tpl>'
		),
	    }],

	    tbar: [
		{
		    text: gettext('Reload'),
		    iconCls: 'fa fa-refresh',
		    handler: 'reload',
		},
	    ],
	});

	me.callParent();
    },
});
