Ext.define('pbs-data-store-snapshots', {
    extend: 'Ext.data.Model',
    fields: [
	'backup-type',
	'backup-id',
	{
	    name: 'backup-time',
	    type: 'date',
	    dateFormat: 'timestamp'
	},
	'files',
	{ name: 'size', type: 'int' },
    ]
});

Ext.define('PBS.DataStoreContent', {
    extend: 'Ext.tree.Panel',
    alias: 'widget.pbsDataStoreContent',

    rootVisible: false,

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    if (!view.datastore) {
		throw "no datastore specified";
	    }

	    this.data_store = Ext.create('Ext.data.Store', {
		model: 'pbs-data-store-snapshots',
		sorters: 'backup-group',
		groupField: 'backup-group',
	    });

	    view.title = gettext('Data Store Content: ') + view.datastore;

	    Proxmox.Utils.monStoreErrors(view, view.store, true);
	    this.reload(); // initial load
	},

	reload: function() {
	    var view = this.getView();

	    let url = `/api2/json/admin/datastore/${view.datastore}/snapshots`;
	    this.data_store.setProxy({
		type: 'proxmox',
		url:  url
	    });

	    this.data_store.load(function(records, operation, success) {
		let groups = {};

		records.forEach(function(item) {
		    var btype = item.data["backup-type"];
		    let group = btype + "/" + item.data["backup-id"];

		    if (groups[group] !== undefined)
			return;

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

		    groups[group] = {
			text: group,
			leaf: false,
			iconCls: "fa " + cls,
			expanded: false,
			children: []
		    };
		});

		records.forEach(function(item) {
		    let group = item.data["backup-type"] + "/" + item.data["backup-id"];
		    let children = groups[group].children;

		    let data = item.data;
		    data.text = Ext.Date.format(data["backup-time"], 'Y-m-d H:i:s');
		    data.leaf = true;

		    children.push(data);
		});

		let children = [];
		Ext.Object.each(groups, function(key, group) {
		    let last_backup = 0;
		    group.children.forEach(function(item) {
			if (item["backup-time"] > last_backup) {
			    last_backup = item["backup-time"];
			    group["backup-time"] = last_backup;
			    group.files = item.files;
			    group.size = item.size;
			}
		    });
		    group.count = group.children.length;
		    children.push(group)
		})

		view.setRootNode({
		    expanded: true,
		    children: children
		});

	    });

	},
    },

    initComponent: function() {
	var me = this;

	Ext.apply(me, {
	    columns: [
		{
		    xtype: 'treecolumn',
		    header: gettext("Backup Group"),
		    dataIndex: 'text',
		    flex: 1
		},
		{
		    xtype: 'datecolumn',
		    header: gettext('Backup Time'),
		    sortable: true,
		    dataIndex: 'backup-time',
		    format: 'Y-m-d H:i:s',
		    width: 150
		},
		{
		    header: gettext("Size"),
		    sortable: true,
		    dataIndex: 'size',
		    renderer: Proxmox.Utils.format_size,
		},
		{
		    xtype: 'numbercolumn',
		    format: '0',
		    header: gettext("Count"),
		    sortable: true,
		    dataIndex: 'count',
		},
		{
		    header: gettext("Files"),
		    sortable: false,
		    dataIndex: 'files',
		    flex: 4
		}
	    ],

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
