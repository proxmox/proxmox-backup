Ext.define('pbs-file-tree', {
    extend: 'Ext.data.Model',

    fields: [ 'filepath', 'text', 'type', 'size',
	{
	    name: 'mtime',
	    type: 'date',
	    dateFormat: 'timestamp',
	},
	{
	    name: 'iconCls',
	    calculate: function(data) {
		let icon = 'file-o';
		switch (data.type) {
		    case 'b': // block device
			icon = 'cube';
			break;
		    case 'c': // char device
			icon = 'tty';
			break;
		    case 'd':
			icon = data.expanded ? 'folder-open-o' : 'folder-o';
			break;
		    case 'f': //regular file
			icon = 'file-text-o';
			break;
		    case 'h': // hardlink
			icon = 'file-o';
			break;
		    case 'l': // softlink
			icon = 'link';
			break;
		    case 'p': // pipe/fifo
			icon = 'exchange';
			break;
		    case 's': // socket
			icon = 'plug';
			break;
		    default:
			icon = 'file-o';
			break;
		}

		return `fa fa-${icon}`;
	    },
	}
    ],
    idProperty: 'filepath',
});

Ext.define("PBS.window.FileBrowser", {
    extend: "Ext.window.Window",

    width: 800,
    height: 600,

    modal: true,

    controller: {
	xclass: 'Ext.app.ViewController',

	buildUrl: function(baseurl, params) {
	    let url = new URL(baseurl, window.location.origin);
	    for (const [key, value] of Object.entries(params)) {
		url.searchParams.append(key, value);
	    }

	    return url.href;
	},

	downloadFile: function() {
	    let me = this;
	    let view = me.getView();
	    let tree = me.lookup('tree');
	    let selection = tree.getSelection();
	    if (!selection || selection.length < 1) return;

	    let data = selection[0].data;

	    let atag = document.createElement('a');

	    atag.download = data.text;
	    let params = {
		'backup-id': view['backup-id'],
		'backup-type': view['backup-type'],
		'backup-time': view['backup-time'],
	    };
	    params['filepath'] = data.filepath;
	    atag.download = data.text;
	    atag.href = me.buildUrl(`/api2/json/admin/datastore/${view.datastore}/pxar-file-download`, params);
	    atag.click();
	},

	fileChanged: function() {
	    let me = this;
	    let view = me.getView();
	    let tree = me.lookup('tree');
	    let selection = tree.getSelection();
	    if (!selection || selection.length < 1) return;

	    let data = selection[0].data;

	    let canDownload = false;
	    switch (data.type) {
		case 'h':
		case 'f':
		    canDownload = true;
		    break;
		default: break;
	    }

	    me.lookup('downloadBtn').setDisabled(!canDownload);
	},

	init: function(view) {
	    let me = this;
	    let tree = me.lookup('tree');

	    if (!view['backup-id']) {
		throw "no backup-id given";
	    }

	    if (!view['backup-type']) {
		throw "no backup-id given";
	    }

	    if (!view['backup-time']) {
		throw "no backup-id given";
	    }

	    if (!view.datastore) {
		throw "no datastore given";
	    }

	    let store = tree.getStore();
	    let proxy = store.getProxy();

	    Proxmox.Utils.monStoreErrors(tree, store, true);
	    proxy.setUrl(`/api2/json/admin/datastore/${view.datastore}/catalog`);
	    proxy.setExtraParams({
		'backup-id': view['backup-id'],
		'backup-type': view['backup-type'],
		'backup-time': view['backup-time'],
	    });
	    store.load();
	    store.getRoot().expand();
	},

	control: {
	    'treepanel': {
		selectionchange: 'fileChanged',
	    },
	},
    },

    layout: 'fit',
    items: [
	{
	    xtype: 'treepanel',
	    scrollable: true,
	    rootVisible: false,
	    reference: 'tree',
	    store: {
		autoLoad: false,
		model: 'pbs-file-tree',
		nodeParam: 'filepath',
		sorters: 'text',
		proxy: {
		    appendId: false,
		    type: 'proxmox',
		},
	    },

	    columns: [
		{
		    text: gettext('Name'),
		    xtype: 'treecolumn',
		    flex: 1,
		    dataIndex: 'text',
		    renderer: Ext.String.htmlEncode,
		},
		{
		    text: gettext('Size'),
		    dataIndex: 'size',
		    renderer: value => value === undefined ? '' : Proxmox.Utils.format_size(value),
		    sorter: {
			sorterFn: function(a, b) {
			    let asize = a.data.size || 0;
			    let bsize = b.data.size || 0;

			    return asize - bsize;
			},
		    }
		},
		{
		    text: gettext('Modified'),
		    dataIndex: 'mtime',
		    minWidth: 200,
		},
		{
		    text: gettext('Type'),
		    dataIndex: 'type',
		    renderer: function(value) {
			switch (value) {
			    case 'b': return gettext('Block Device');
			    case 'c': return gettext('Character Device');
			    case 'd': return gettext('Directory');
			    case 'f': return gettext('File');
			    case 'h': return gettext('Hardlink');
			    case 'l': return gettext('Softlink');
			    case 'p': return gettext('Pipe/Fifo');
			    case 's': return gettext('Socket');
			    default: return Proxmox.Utils.unknownText;
			}
		    }
		},
	    ]
	}
    ],

    buttons: [
	{
	    text: gettext('Download'),
	    handler: 'downloadFile',
	    reference: 'downloadBtn',
	    disabled: true,
	}
    ],
});
