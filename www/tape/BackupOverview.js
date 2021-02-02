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
	    let uuid = selection[0].data.uuid;
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
	    let content_response = await PBS.Async.api2({
		url: '/api2/extjs/tape/media/content',
	    });
	    let data = {};

	    for (const entry of content_response.result.data) {
		let pool = entry.pool;
		let [type, group_id, id] = PBS.Utils.parse_snapshot_id(entry.snapshot);
		let group = `${type}/${group_id}`;
		let media_set = entry['media-set-name'];
		let uuid = entry['media-set-uuid'];
		let ctime = entry['media-set-ctime'];
		if (data[pool] === undefined) {
		    data[pool] = {};
		}

		if (data[pool][group] === undefined) {
		    data[pool][group] = {};
		}

		if (data[pool][group][id] === undefined) {
		    data[pool][group][id] = [];
		}
		data[pool][group][id].push({
		    text: media_set,
		    uuid,
		    ctime,
		    leaf: true,
		});
	    }

	    let list = [];

	    for (const [pool, groups] of Object.entries(data)) {
		let pool_entry = {
		    text: pool,
		    leaf: false,
		    children: [],
		};
		for (const [group, ids] of Object.entries(groups)) {
		    let group_entry = {
			text: group,
			iconCls: "fa " + PBS.Utils.get_type_icon_cls(group),
			leaf: false,
			children: [],
		    };
		    for (const [id, media_sets] of Object.entries(ids)) {
			let id_entry = {
			    text: `${group}/${id}`,
			    leaf: false,
			    children: media_sets,
			};
			group_entry.children.push(id_entry);
		    }
		    pool_entry.children.push(group_entry);
		}
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
    },

    listeners: {
	activate: 'reload',
    },

    store: {
	data: [],
	sorters: function(a, b) {
	    if (a.data.leaf && b.data.leaf) {
		return a.data.ctime - b.data.ctime;
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
	    text: gettext('Pool/Group/Snapshot/Media Set'),
	    dataIndex: 'text',
	    sortable: false,
	    flex: 3,
	},
	{
	    text: gettext('Media Set UUID'),
	    dataIndex: 'uuid',
	    sortable: false,
	    flex: 1,
	},
    ],
});

