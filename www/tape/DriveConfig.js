Ext.define('pbs-model-drives', {
    extend: 'Ext.data.Model',
    fields: [
	'path', 'model', 'name', 'serial', 'vendor',
	{ name: 'changer', defaultValue: '' },
	{
	    name: 'changer-drivenum',
	    defaultValue: 0,
	},
	'changer-slot',
    ],
    idProperty: 'name',
});

Ext.define('PBS.TapeManagement.DrivePanel', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.pbsTapeDrivePanel',

    controller: {
	xclass: 'Ext.app.ViewController',

	onAdd: function() {
	    let me = this;
	    Ext.create('PBS.TapeManagement.DriveEditWindow', {
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	onEdit: function() {
	    let me = this;
	    let view = me.getView();
	    let selection = view.getSelection();
	    if (!selection || selection.length < 1) {
		return;
	    }
	    Ext.create('PBS.TapeManagement.DriveEditWindow', {
		driveid: selection[0].data.name,
		autoLoad: true,
		listeners: {
		    destroy: () => me.reload(),
		},
	    }).show();
	},

	status: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    PBS.Utils.driveCommand(drive, 'status', {
		waitMsgTarget: me.getView(),
		success: PBS.Utils.showDriveStatusWindow,
	    });
	},

	readLabel: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;

	    PBS.Utils.driveCommand(drive, 'read-label', {
		waitMsgTarget: me.getView(),
		success: PBS.Utils.showMediaLabelWindow,
	    });
	},

	volumeStatistics: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    PBS.Utils.driveCommand(drive, 'volume-statistics', {
		waitMsgTarget: me.getView(),
		success: PBS.Utils.showVolumeStatisticsWindow,
	    });
	},

	cartridgeMemory: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    PBS.Utils.driveCommand(drive, 'cartridge-memory', {
		waitMsgTarget: me.getView(),
		success: PBS.Utils.showCartridgeMemoryWindow,
	    });
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

	init: function(view) {
	    Proxmox.Utils.monStoreErrors(view, view.getStore().rstore);
	},
    },

    listeners: {
	beforedestroy: 'stopStore',
	deactivate: 'stopStore',
	activate: 'startStore',
	itemdblclick: 'onEdit',
    },

    store: {
	type: 'diff',
	rstore: {
	    type: 'update',
	    storeid: 'proxmox-tape-drives',
	    model: 'pbs-model-drives',
	    proxy: {
		type: 'proxmox',
		url: "/api2/json/tape/drive",
	    },
	},
	sorters: 'name',
	groupField: 'changer',
    },

    features: [
	{
	    ftype: 'grouping',
	    groupHeaderTpl: [
		'{name:this.formatName} ({rows.length} Item{[values.rows.length > 1 ? "s" : ""]})',
		{
		    formatName: function(changer) {
			if (changer === "") {
			    return "Standalone Drives";
			} else {
			    return `Changer ${changer}`;
			}
		    },
		},
	    ],
	},
    ],

    tbar: [
	{
	    text: gettext('Add'),
	    xtype: 'proxmoxButton',
	    handler: 'onAdd',
	    selModel: false,
	},
	'-',
	{
	    text: gettext('Edit'),
	    xtype: 'proxmoxButton',
	    handler: 'onEdit',
	    disabled: true,
	},
	{
	    xtype: 'proxmoxStdRemoveButton',
	    baseurl: '/api2/extjs/config/drive',
	    callback: 'reload',
	},
    ],
    columns: [
	{
	    text: gettext('Name'),
	    dataIndex: 'name',
	    flex: 1,
	},
	{
	    text: gettext('Path'),
	    dataIndex: 'path',
	    flex: 2,
	},
	{
	    text: gettext('Vendor'),
	    dataIndex: 'vendor',
	    flex: 1,
	},
	{
	    text: gettext('Model'),
	    dataIndex: 'model',
	    flex: 1,
	},
	{
	    text: gettext('Serial'),
	    dataIndex: 'serial',
	    flex: 1,
	},
	{
	    text: gettext('Drive Number'),
	    dataIndex: 'changer-drivenum',
	    renderer: function(value, mD, record) {
		return record.data.changer ? value : '';
	    },
	},
	{
	    text: gettext('Actions'),
	    width: 140,
	    xtype: 'actioncolumn',
	    items: [
		{
		    iconCls: 'fa fa-hdd-o',
		    handler: 'cartridgeMemory',
		    tooltip: gettext('Cartridge Memory'),
		},
		{
		    iconCls: 'fa fa-line-chart',
		    handler: 'volumeStatistics',
		    tooltip: gettext('Volume Statistics'),
		},
		{
		    iconCls: 'fa fa-tag',
		    handler: 'readLabel',
		    tooltip: gettext('Read Label'),
		},
		{
		    iconCls: 'fa fa-info-circle',
		    handler: 'status',
		    tooltip: gettext('Status'),
		},
	    ],
	},
    ],
});

