Ext.define('pbs-model-tapes', {
    extend: 'Ext.data.Model',
    fields: [
	'catalog',
	'ctime',
	'expired',
	'label-text',
	'location',
	'media-set-ctime',
	'media-set-name',
	'media-set-uuid',
	'pool',
	'seq-nr',
	'status',
	'uuid',
    ],
    idProperty: 'label-text',
    proxy: {
	type: 'proxmox',
	url: '/api2/json/tape/media/list',
    },
});

Ext.define('PBS.TapeManagement.TapeInventory', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.pbsTapeInventory',

    controller: {
	xclass: 'Ext.app.ViewController',

	moveToVault: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (!selection || selection.length < 1) {
		return;
	    }
	    let label = selection[0].data['label-text'];
	    let inVault = selection[0].data.location.startsWith('vault-');
	    let vault = "";
	    if (inVault) {
		vault = selection[0].data.location.slice("vault-".length);
	    }
	    Ext.create('Proxmox.window.Edit', {
		title: gettext('Set Tape Location'),
		url: `/api2/extjs/tape/media/move`,
		method: 'POST',
		items: [
		    {
			xtype: 'displayfield',
			name: 'label-text',
			value: label,
			submitValue: true,
			fieldLabel: gettext('Media'),
		    },
		    {
			xtype: 'proxmoxtextfield',
			fieldLabel: gettext('Vault'),
			name: 'vault-name',
			value: vault,
			emptyText: gettext('On-site'),
			skipEmpty: true,
		    },
		],
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	reload: function() {
	    this.getView().getStore().rstore.load();
	},

	stopStore: function() {
	    this.getView().getStore().rstore.stopUpdate();
	},

	startStore: function() {
	    this.getView().getStore().rstore.startUpdate();
	},
    },

    listeners: {
	beforedestroy: 'stopStore',
	deactivate: 'stopStore',
	activate: 'startStore',
    },

    store: {
	type: 'diff',
	rstore: {
	    type: 'update',
	    storeid: 'proxmox-tape-tapes',
	    model: 'pbs-model-tapes',
	},
	sorters: 'label-text',
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Set Tape Location'),
	    disabled: true,
	    handler: 'moveToVault',
	    enableFn: (rec) => !rec.data.location.startsWith('online-'),
	},
    ],

    columns: [
	{
	    text: gettext('Label'),
	    dataIndex: 'label-text',
	    flex: 1,
	},
	{
	    text: gettext('Pool'),
	    dataIndex: 'pool',
	    sorter: (a, b) => (a.data.pool || "").localeCompare(b.data.pool || ""),
	    flex: 1,
	},
	{
	    text: gettext('Media Set'),
	    dataIndex: 'media-set-name',
	    flex: 2,
	    sorter: function(a, b) {
		return (a.data['media-set-ctime'] || 0) - (b.data['media-set-ctime'] || 0);
	    },
	},
	{
	    text: gettext('Location'),
	    dataIndex: 'location',
	    flex: 1,
	    renderer: function(value) {
		if (value === 'offline') {
		    return `<i class="fa fa-circle-o"></i> ${gettext("Offline")} (${gettext('On-site')})`;
		} else if (value.startsWith('online-')) {
		    let location = value.substring(value.indexOf('-') + 1);
		    return `<i class="fa fa-dot-circle-o"></i> ${gettext("Online")} - ${location}`;
		} else if (value.startsWith('vault-')) {
		    let location = value.substring(value.indexOf('-') + 1);
		    return `<i class="fa fa-archive"></i> ${gettext("Vault")} - ${location}`;
		} else {
		    return value;
		}
	    },
	},
	{
	    text: gettext('Status'),
	    dataIndex: 'status',
	    renderer: function(value, mD, record) {
		return record.data.expired ? 'expired' : value;
	    },
	    flex: 1,
	},
    ],
});
