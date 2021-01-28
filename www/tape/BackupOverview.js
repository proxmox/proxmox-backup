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

	reload: async function() {
	    let me = this;
	    let view = me.getView();

	    Proxmox.Utils.setErrorMask(view, true);

	    try {
		let list_response = await PBS.Async.api2({
		    url: '/api2/extjs/tape/media/list',
		});
		let list = list_response.result.data.sort(
		    (a, b) => a['label-text'].localeCompare(b['label-text']),
		);

		let content = {};

		let content_response = await PBS.Async.api2({
		    url: '/api2/extjs/tape/media/content',
		});

		let content_list = content_response.result.data.sort(
		    (a, b) => a.snapshot.localeCompare(b.snapshot),
		);

		for (let entry of content_list) {
		    let tape = entry['label-text'];
		    entry['label-text'] = entry.snapshot;
		    entry.leaf = true;
		    if (content[tape] === undefined) {
			content[tape] = [entry];
		    } else {
			content[tape].push(entry);
		    }
		}

		for (let child of list) {
		    let tape = child['label-text'];
		    if (content[tape]) {
			child.children = content[tape];
			child.leaf = false;
		    } else {
			child.leaf = true;
		    }
		}

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
	sorters: 'label-text',
	data: [],
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
    ],

    columns: [
	{
	    xtype: 'treecolumn',
	    text: gettext('Tape/Backup'),
	    dataIndex: 'label-text',
	    flex: 3,
	},
	{
	    text: gettext('Location'),
	    dataIndex: 'location',
	    flex: 1,
	    renderer: function(value) {
		if (!value) {
		    return "";
		}
		let result;
		if ((result = /^online-(.+)$/.exec(value)) !== null) {
		    return Ext.htmlEncode(result[1]);
		}

		return value;
	    },
	},
	{
	    text: gettext('Status'),
	    dataIndex: 'status',
	    flex: 1,
	},
	{
	    text: gettext('Media Set'),
	    dataIndex: 'media-set-name',
	    flex: 2,
	},
	{
	    text: gettext('Pool'),
	    dataIndex: 'pool',
	    flex: 1,
	},
	{
	    text: gettext('Sequence Nr.'),
	    dataIndex: 'seq-nr',
	    flex: 0.5,
	},
	{
	    text: gettext('Backup Time'),
	    dataIndex: 'backup-time',
	    renderer: (time) => time !== undefined ? new Date(time*1000) : "",
	    flex: 1,
	},
    ],
});

