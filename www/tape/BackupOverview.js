Ext.define('PBS.TapeManagement.BackupOverview', {
    extend: 'Ext.tree.Panel',
    alias: 'widget.pbsBackupOverview',

    controller: {
	xclass: 'Ext.app.ViewController',

	backup: function() {
	    let me = this;
	    Ext.create('PBS.TapeManagement.TapeBackupWindow', {
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	restore: function(button, record) {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (!selection || selection.length < 1) {
		return;
	    }

	    let node = selection[0];
	    let mediaset = node.data.text;
	    let uuid = node.data['media-set-uuid'];
	    let datastores = node.data.datastores;
	    while (!datastores && node.get('depth') > 2) {
		node = node.parentNode;
		datastores = node.data.datastores;
	    }
	    Ext.create('PBS.TapeManagement.TapeRestoreWindow', {
		mediaset,
		uuid,
		datastores,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	loadContent: async function() {
	    let me = this;
	    let content_response = await PBS.Async.api2({
		url: '/api2/extjs/tape/media/list?update-status=false',
	    });
	    let data = {};

	    for (const entry of content_response.result.data) {
		let pool = entry.pool;
		if (pool === undefined) {
		    continue; // pools not belonging to a pool cannot contain data
		}
		let media_set = entry['media-set-name'];
		if (media_set === undefined) {
		    continue; // tape does not belong to media-set (yet))
		}
		if (data[pool] === undefined) {
		    data[pool] = {};
		}

		if (data[pool][media_set] === undefined) {
		    data[pool][media_set] = entry;
		    data[pool][media_set].text = media_set;
		    data[pool][media_set].tapes = 1;
		    data[pool][media_set]['seq-nr'] = undefined;
		    data[pool][media_set].is_media_set = true;
		} else {
		    data[pool][media_set].tapes++;
		}
	    }

	    let list = [];

	    for (const [pool, media_sets] of Object.entries(data)) {
		let pool_entry = Ext.create('Ext.data.TreeModel', {
		    text: pool,
		    leaf: false,
		});

		let children = [];

		for (const media_set of Object.values(media_sets)) {
		    let entry = Ext.create('Ext.data.TreeModel', media_set);
		    entry.on('beforeexpand', (node) => me.beforeExpand(node));
		    children.push(entry);
		}

		pool_entry.set('children', children);
		list.push(pool_entry);
	    }

	    return list;
	},

	reload: async function() {
	    let me = this;
	    let view = me.getView();

	    Proxmox.Utils.setErrorMask(view, true);

	    try {
		let list = await me.loadContent();

		view.setRootNode({
		    expanded: true,
		    children: list,
		});

		Proxmox.Utils.setErrorMask(view, false);
	    } catch (error) {
		Proxmox.Utils.setErrorMask(view, error.toString());
	    }
	},

	loadMediaSet: async function(node) {
	    let me = this;
	    let view = me.getView();

	    Proxmox.Utils.setErrorMask(view, true);
	    const media_set = node.data['media-set-uuid'];

	    try {
		let list = await PBS.Async.api2({
		    method: 'GET',
		    url: `/api2/extjs/tape/media/content`,
		    params: {
			'media-set': media_set,
		    },
		});

		list.result.data.sort(function(a, b) {
		    let storeRes = a.store.localeCompare(b.store);
		    if (storeRes === 0) {
			return a.snapshot.localeCompare(b.snapshot);
		    } else {
			return storeRes;
		    }
		});

		let stores = {};

		for (let entry of list.result.data) {
		    entry.text = entry.snapshot;
		    entry.leaf = true;
		    entry.children = [];
		    let iconCls = PBS.Utils.get_type_icon_cls(entry.snapshot);
		    if (iconCls !== '') {
			entry.iconCls = `fa ${iconCls}`;
		    }

		    let store = entry.store;
		    let tape = entry['label-text'];
		    if (stores[store] === undefined) {
			stores[store] = {
			    text: store,
			    'media-set-uuid': entry['media-set-uuid'],
			    iconCls: 'fa fa-database',
			    tapes: {},
			};
		    }

		    if (stores[store].tapes[tape] === undefined) {
			stores[store].tapes[tape] = {
			    text: tape,
			    'media-set-uuid': entry['media-set-uuid'],
			    'seq-nr': entry['seq-nr'],
			    iconCls: 'pbs-icon-tape',
			    expanded: true,
			    children: [],
			};
		    }
		    let [type, group, _id] = PBS.Utils.parse_snapshot_id(entry.snapshot);

		    let children = stores[store].tapes[tape].children;
		    let text = `${type}/${group}`;
		    if (children.length < 1 || children[children.length - 1].text !== text) {
			children.push({
			    text,
			    'media-set-uuid': entry['media-set-uuid'],
			    leaf: false,
			    iconCls: `fa ${iconCls}`,
			    children: [],
			});
		    }
		    children[children.length - 1].children.push(entry);
		}

		let storeList = Object.values(stores);
		let storeNameList = Object.keys(stores);
		let expand = storeList.length === 1;
		for (const store of storeList) {
		    store.children = Object.values(store.tapes);
		    store.expanded = expand;
		    delete store.tapes;
		    node.appendChild(store);
		}

		if (list.result.data.length === 0) {
		    node.set('leaf', true);
		}

		node.set('loaded', true);
		node.set('datastores', storeNameList);
		Proxmox.Utils.setErrorMask(view, false);
		node.expand();
	    } catch (error) {
		Proxmox.Utils.setErrorMask(view, false);
		Ext.Msg.alert('Error', error.toString());
	    }
	},

	beforeExpand: function(node, e) {
	    let me = this;
	    if (node.isLoaded()) {
		return true;
	    }

	    me.loadMediaSet(node);

	    return false;
	},
    },

    listeners: {
	activate: 'reload',
    },

    store: {
	data: [],
	sorters: function(a, b) {
	    if (a.data.is_media_set && b.data.is_media_set) {
		return a.data['media-set-ctime'] - b.data['media-set-ctime'];
	    } else {
		return a.data.text.localeCompare(b.data.text);
	    }
	},
    },

    rootVisible: false,

    tbar: [
	{
	    text: gettext('Reload'),
	    handler: 'reload',
	},
	'-',
	{
	    text: gettext('New Backup'),
	    handler: 'backup',
	},
	{
	    xtype: 'proxmoxButton',
	    disabled: true,
	    text: gettext('Restore Media Set'),
	    handler: 'restore',
	    parentXType: 'treepanel',
	    enableFn: (rec) => !!rec.data['media-set-uuid'],
	},
    ],

    columns: [
	{
	    xtype: 'treecolumn',
	    text: gettext('Pool/Media Set/Snapshot'),
	    dataIndex: 'text',
	    sortable: false,
	    flex: 3,
	},
	{
	    text: gettext('Tapes'),
	    dataIndex: 'tapes',
	    sortable: false,
	},
	{
	    text: gettext('Seq. Nr.'),
	    dataIndex: 'seq-nr',
	    sortable: false,
	},
	{
	    text: gettext('Media Set UUID'),
	    dataIndex: 'media-set-uuid',
	    hidden: true,
	    sortable: false,
	    width: 280,
	},
    ],
});

