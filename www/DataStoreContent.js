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

    title: gettext('Content'),

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    if (!view.datastore) {
		throw "no datastore specified";
	    }

	    this.store = Ext.create('Ext.data.Store', {
		model: 'pbs-data-store-snapshots',
		sorters: 'backup-group',
		groupField: 'backup-group',
	    });
	    this.store.on('load', this.onLoad, this);

	    Proxmox.Utils.monStoreErrors(view, view.store, true);
	    this.reload(); // initial load
	},

	reload: function() {
	    let view = this.getView();

	    if (!view.store || !this.store) {
		console.warn('cannot reload, no store(s)');
		return;
	    }

	    let url = `/api2/json/admin/datastore/${view.datastore}/snapshots`;
	    this.store.setProxy({
		type: 'proxmox',
		url:  url
	    });

	    this.store.load();
	},

	getRecordGroups: function(records) {
	    let groups = {};

	    for (const item of records) {
		var btype = item.data["backup-type"];
		let group = btype + "/" + item.data["backup-id"];

		if (groups[group] !== undefined) {
		    continue;
		}

		var cls = '';
		if (btype === 'vm') {
		    cls = 'fa-desktop';
		} else if (btype === 'ct') {
		    cls = 'fa-cube';
		} else if (btype === 'host') {
		    cls = 'fa-building';
		} else {
		    console.warn(`got unkown backup-type '${btype}'`);
		    continue; // FIXME: auto render? what do?
		}

		groups[group] = {
		    text: group,
		    leaf: false,
		    iconCls: "fa " + cls,
		    expanded: false,
		    backup_type: item.data["backup-type"],
		    backup_id: item.data["backup-id"],
		    children: []
		};
	    }

	    return groups;
	},

	onLoad: function(store, records, success) {
	    let view = this.getView();

	    if (!success) {
		return;
	    }

	    let groups = this.getRecordGroups(records);

	    let backup_time_to_string = function(backup_time) {
		let pad = (number) => number < 10 ? '0' + number : number;
		return backup_time.getUTCFullYear() +
		    '-' + pad(backup_time.getUTCMonth() + 1) +
		    '-' + pad(backup_time.getUTCDate()) +
		    'T' + pad(backup_time.getUTCHours()) +
		    ':' + pad(backup_time.getUTCMinutes()) +
		    ':' + pad(backup_time.getUTCSeconds()) +
		    'Z';
	    };

	    for (const item of records) {
		let group = item.data["backup-type"] + "/" + item.data["backup-id"];
		let children = groups[group].children;

		let data = item.data;

		data.text = Ext.Date.format(data["backup-time"], 'Y-m-d H:i:s');
		data.text = group + '/' + backup_time_to_string(data["backup-time"]);
		data.leaf = true;
		data.cls = 'no-leaf-icons';

		children.push(data);
	    }

	    let children = [];
	    for (const [_key, group] of Object.entries(groups)) {
		let last_backup = 0;
		for (const item of group.children) {
		    if (item["backup-time"] > last_backup) {
			last_backup = item["backup-time"];
			group["backup-time"] = last_backup;
			group.files = item.files;
			group.size = item.size;
		    }
		}
		group.count = group.children.length;
		children.push(group);
	    }

	    view.setRootNode({
		expanded: true,
		children: children
	    });
	},

	onPrune: function() {
	    var view = this.getView();

	    let rec = view.selModel.getSelection()[0];
	    if (!(rec && rec.data)) return;
	    let data = rec.data;
	    if (data.leaf) return;

	    if (!view.datastore) return;

	    let win = Ext.create('PBS.DataStorePrune', {
		datastore: view.datastore,
		backup_type: data.backup_type,
		backup_id: data.backup_id,
	    });
	    win.on('destroy', this.reload, this);
	    win.show();
	}
    },

    initComponent: function() {
	var me = this;

	var sm = Ext.create('Ext.selection.RowModel', {});

	var prune_btn = new Proxmox.button.Button({
	    text: gettext('Prune'),
	    disabled: true,
	    selModel: sm,
	    enableFn: function(record) { return !record.data.leaf; },
	    handler: 'onPrune',
	});

	Ext.apply(me, {
	    selModel: sm,
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
		    flex: 2
		}
	    ],

	    tbar: [
		{
		    text: gettext('Reload'),
		    iconCls: 'fa fa-refresh',
		    handler: 'reload',
		},
		prune_btn
	    ],
	});

	me.callParent();
    },
});
