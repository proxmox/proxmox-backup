Ext.define('pbs-model-tapes', {
    extend: 'Ext.data.Model',
    fields: [
	{ name: 'catalog', type: 'boolean' },
	'ctime',
	{ name: 'expired', type: 'boolean' },
	'label-text',
	'location',
	'media-set-ctime',
	'media-set-name',
	'media-set-uuid',
	{
	    name: 'pool',
	    defaultValue: '',
	},
	'seq-nr',
	'status',
	'uuid',
    ],
    idProperty: 'uuid',
    proxy: {
	type: 'proxmox',
	url: '/api2/json/tape/media/list',
	timeout: 5*60*1000,
    },
});

Ext.define('PBS.TapeManagement.TapeInventory', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.pbsTapeInventory',

    controller: {
	xclass: 'Ext.app.ViewController',

	addTape: function() {
	    Ext.create('PBS.TapeManagement.LabelMediaWindow').show();
	},

	format: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (!selection || selection.length < 1) {
		return;
	    }
	    let label = selection[0].data['label-text'];
	    let inChanger = selection[0].data.location.startsWith('online-');
	    let changer;
	    if (inChanger) {
		changer = selection[0].data.location.slice("online-".length);
	    }
	    Ext.create('PBS.TapeManagement.EraseWindow', {
		label,
		changer,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	remove: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (!selection || selection.length < 1) {
		return;
	    }
	    let uuid = selection[0].data.uuid;
	    let label = selection[0].data['label-text'];
	    Ext.create('PBS.TapeManagement.MediaRemoveWindow', {
		uuid,
		label,
		autoShow: true,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    });
	},

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
		title: gettext('Set Media Location'),
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

	setStatus: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (!selection || selection.length < 1) {
		return;
	    }
	    let data = selection[0].data;

	    let uuid = data.uuid;
	    let label = data['label-text'];
	    let status = data.status;

	    Ext.create('Proxmox.window.Edit', {
		title: gettext('Set Media Status'),
		url: `/api2/extjs/tape/media/list/${uuid}/status`,
		method: 'POST',
		items: [
		    {
			xtype: 'displayfield',
			name: 'label-text',
			value: label,
			fieldLabel: gettext('Media'),
		    },
		    {
			xtype: 'proxmoxKVComboBox',
			fieldLabel: gettext('Status'),
			name: 'status',
			value: status,
			emptyText: gettext('Clear Status'),
			comboItems: [
			    ['__default__', gettext('Clear Status')],
			    ['full', gettext('Full')],
			    ['damaged', gettext('Damaged')],
			    ['retired', gettext('Retired')],
			],
			deleteEmpty: false,
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
	    this.getView().getStore().load({
		params: { 'update-status': false },
	    });
	},

	reload_update_status: function() {
	    this.getView().getStore().load({
		params: { 'update-status': true },
	    });
	},

	init: function(view) {
	    Proxmox.Utils.monStoreErrors(view, view.getStore());
	},
    },

    listeners: {
	activate: 'reload',
    },

    store: {
	storeid: 'proxmox-tape-tapes',
	model: 'pbs-model-tapes',
	sorters: 'label-text',
	groupField: 'pool',
    },

    tbar: [
	{
	    text: gettext('Reload'),
	    handler: 'reload_update_status',
	},
	'-',
	{
	    text: gettext('Add Tape'),
	    handler: 'addTape',
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Set Location'),
	    disabled: true,
	    handler: 'moveToVault',
	    enableFn: (rec) => !rec.data.location.startsWith('online-'),
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Set Status'),
	    disabled: true,
	    handler: 'setStatus',
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Format'),
	    disabled: true,
	    handler: 'format',
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Remove'),
	    disabled: true,
	    handler: 'remove',
	},
    ],

    features: [
	{
	    ftype: 'grouping',
	    groupHeaderTpl: [
		'{name:this.formatName} ({rows.length} Item{[values.rows.length > 1 ? "s" : ""]})',
		{
		    formatName: function(pool) {
			if (pool === "") {
			    return "Free (no pool assignment)";
			} else {
			    return pool;
			}
		    },
		},
	    ],
	},
    ],

    viewConfig: {
	stripeRows: false, // does not work with getRowClass()

	getRowClass: function(record, index) {
	    let status = record.get('status');
	    if (status === 'damaged') {
		return "proxmox-invalid-row";
	    }
	    let catalog = record.get('catalog');
	    return catalog ? '' : "proxmox-warning-row";
	},
    },

    columns: [
	{
	    text: gettext('Label'),
	    dataIndex: 'label-text',
	    flex: 1,
	},
	{
	    text: gettext('Media-Set'),
	    dataIndex: 'media-set-name',
	    flex: 2,
	    sorter: function(a, b) {
		return (a.data['media-set-ctime'] || 0) - (b.data['media-set-ctime'] || 0);
	    },
	    renderer: function(value) {
		if (value === undefined) {
		    return "-- empty --";
		} else {
		    return value;
		}
	    },
	},
	{
	    text: gettext('Catalog'),
	    dataIndex: 'catalog',
	    renderer: function(value, metaData, record) {
		return value ? Proxmox.Utils.yesText : PBS.Utils.missingText;
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
	{
	    text: gettext('UUID'),
	    dataIndex: 'uuid',
	    flex: 1,
	    hidden: true,
	},
    ],
});
