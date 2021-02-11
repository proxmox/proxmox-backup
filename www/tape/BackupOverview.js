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

	    let mediaset = selection[0].data.text;
	    let uuid = selection[0].data['media-set-uuid'];
	    Ext.create('PBS.TapeManagement.TapeRestoreWindow', {
		mediaset,
		uuid,
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
		url: '/api2/extjs/tape/media/list',
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

		list.result.data.sort((a, b) => a.snapshot.localeCompare(b.snapshot));

		for (let entry of list.result.data) {
		    entry.text = entry.snapshot;
		    entry.leaf = true;
		    entry.children = [];
		    let iconCls = PBS.Utils.get_type_icon_cls(entry.snapshot);
		    if (iconCls !== '') {
			entry.iconCls = `fa ${iconCls}`;
		    }
		    node.appendChild(entry);
		}

		if (list.result.data.length === 0) {
		    node.set('leaf', true);
		}

		node.set('loaded', true);
		Proxmox.Utils.setErrorMask(view, false);
		node.expand();
	    } catch (error) {
		Proxmox.Utils.setErrorMask(view, error.toString());
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
	    enableFn: (rec) => !!rec.data.uuid,
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
	    text: gettext('Number of Tapes'),
	    dataIndex: 'tapes',
	    sortable: false,
	    flex: 1,
	},
	{
	    text: gettext('Tape Number'),
	    dataIndex: 'seq-nr',
	    sortable: false,
	    flex: 1,
	},
	{
	    text: gettext('Media Set UUID'),
	    dataIndex: 'media-set-uuid',
	    sortable: false,
	    flex: 1,
	},
    ],
});

