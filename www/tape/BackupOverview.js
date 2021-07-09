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
		autoShow: true,
	    });
	},

	restore: function() {
	    Ext.create('PBS.TapeManagement.TapeRestoreWindow', {
		autoShow: true,
	    });
	},

	restoreBackups: function(view, rI, cI, item, e, rec) {
	    let me = this;

	    let mediaset = rec.data.is_media_set ? rec.data.text : rec.data['media-set'];
	    Ext.create('PBS.TapeManagement.TapeRestoreWindow', {
		autoShow: true,
		uuid: rec.data['media-set-uuid'],
		prefilter: rec.data.prefilter,
		mediaset,
	    });
	},

	loadContent: async function() {
	    let me = this;
	    let content_response = await Proxmox.Async.api2({
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
		    data[pool][media_set].restore = true;
		    data[pool][media_set].tapes = 1;
		    data[pool][media_set]['seq-nr'] = undefined;
		    data[pool][media_set].is_media_set = true;
		    data[pool][media_set].typeText = 'media-set';
		} else {
		    data[pool][media_set].tapes++;
		}
	    }

	    let list = [];

	    for (const [pool, media_sets] of Object.entries(data)) {
		let pool_entry = Ext.create('Ext.data.TreeModel', {
		    text: pool,
		    iconCls: 'fa fa-object-group',
		    expanded: true,
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
	    const media_set_uuid = node.data['media-set-uuid'];
	    const media_set = node.data.text;

	    try {
		let list = await Proxmox.Async.api2({
		    method: 'GET',
		    url: `/api2/extjs/tape/media/content`,
		    params: {
			'media-set': media_set_uuid,
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
		    entry.restore = true;
		    entry.leaf = true;
		    entry.children = [];
		    entry['media-set'] = media_set;
		    entry.prefilter = {
			store: entry.store,
			snapshot: entry.snapshot,
		    };
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
			    typeText: 'datastore',
			    restore: true,
			    'media-set': media_set,
			    prefilter: {
				store,
			    },
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
			    restore: true,
			    prefilter: {
				store,
				snapshot: `${type}/${group}/`,
			    },
			    'media-set': media_set,
			    iconCls: `fa ${iconCls}`,
			    typeText: `group`,
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
	    } catch (response) {
		Proxmox.Utils.setErrorMask(view, false);
		Ext.Msg.alert('Error', response.result.message.toString());
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
	    iconCls: 'fa fa-refresh',
	    handler: 'reload',
	},
	'-',
	{
	    text: gettext('New Backup'),
	    iconCls: 'fa fa-floppy-o',
	    handler: 'backup',
	},
	'-',
	{
	    text: gettext('Restore'),
	    iconCls: 'fa fa-undo',
	    handler: 'restore',
	},
    ],

    columns: [
	{
	    xtype: 'treecolumn',
	    text: gettext('Pool/Media-Set/Snapshot'),
	    dataIndex: 'text',
	    sortable: false,
	    flex: 3,
	},
	{
	    header: gettext('Restore'),
	    xtype: 'actioncolumn',
	    dataIndex: 'text',
	    items: [
		{
		    handler: 'restoreBackups',
		    getTip: (v, m, rec) => {
			let typeText = rec.get('typeText');
			if (typeText) {
			    v = `${typeText} '${v}'`;
			}
			return Ext.String.format(gettext("Open restore wizard for {0}"), v);
		    },
		    getClass: (v, m, rec) => rec.data.restore ? 'fa fa-fw fa-undo' : 'pmx-hidden',
		    isActionDisabled: (v, r, c, i, rec) => !rec.data.restore,
                },
	    ],
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
	    text: gettext('Media-Set UUID'),
	    dataIndex: 'media-set-uuid',
	    hidden: true,
	    sortable: false,
	    width: 280,
	},
    ],
});

