Ext.define('PBS.TapeManagement.DriveStatus', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsDriveStatus',
    mixins: ['Proxmox.Mixin.CBind'],

    cbindData: function(config) {
	let me = this;
	me.setTitle(`${gettext('Drive')}: ${me.drive}`);
	return {
	    driveStatusUrl: `/api2/json/tape/drive/${me.drive}/status`,
	};
    },

    scrollable: true,

    bodyPadding: 5,

    viewModel: {
	data: {
	    online: false,
	},
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	reload: function() {
	    let me = this;
	    me.lookup('statusgrid').rstore.load();
	},

	onLoad: function() {
	    let me = this;
	    let statusgrid = me.lookup('statusgrid');
	    let statusFlags = (statusgrid.getObjectValue('status') || "").split(/\s+|\s+/);
	    let online = statusFlags.indexOf('ONLINE') !== -1;
	    let vm = me.getViewModel();
	    vm.set('online', online);
	},

	labelMedia: function() {
	    let me = this;
	    Ext.create('PBS.TapeManagement.LabelMediaWindow', {
		driveid: me.getView().drive,
	    }).show();
	},

	eject: function() {
	    let me = this;
	    let view = me.getView();
	    let driveid = view.drive;
	    PBS.Utils.driveCommand(driveid, 'eject-media', {
		waitMsgTarget: view,
		method: 'POST',
		success: function(response) {
		    Ext.create('Proxmox.window.TaskProgress', {
			upid: response.result.data,
			taskDone: function() {
			    me.reload();
			},
		    }).show();
		},
	    });
	},

	catalog: function() {
	    let me = this;
	    let view = me.getView();
	    let drive = view.drive;
	    PBS.Utils.driveCommand(drive, 'catalog', {
		waitMsgTarget: view,
		method: 'POST',
		success: function(response) {
		    Ext.create('Proxmox.window.TaskViewer', {
			upid: response.result.data,
			taskDone: function() {
			    me.reload();
			},
		    }).show();
		},
	    });
	},

	init: function(view) {
	    let me = this;
	    me.mon(me.lookup('statusgrid').getStore().rstore, 'load', 'onLoad');
	},
    },

    listeners: {
	activate: 'reload',
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    handler: 'reload',
	    text: gettext('Reload'),
	},
	'-',
	{
	    text: gettext('Label Media'),
	    xtype: 'proxmoxButton',
	    handler: 'labelMedia',
	    iconCls: 'fa fa-barcode',
	    disabled: true,
	    bind: {
		disabled: '{!online}',
	    },
	},
	{
	    text: gettext('Eject'),
	    xtype: 'proxmoxButton',
	    handler: 'ejectMedia',
	    iconCls: 'fa fa-eject',
	    disabled: true,
	    bind: {
		disabled: '{!online}',
	    },
	},
	{
	    text: gettext('Catalog'),
	    xtype: 'proxmoxButton',
	    handler: 'catalog',
	    iconCls: 'fa fa-book',
	    disabled: true,
	    bind: {
		disabled: '{!online}',
	    },
	},
    ],

    items: [
	{
	    xtype: 'container',
	    layout: {
		type: 'hbox',
		align: 'stretch',
	    },
	    defaults: {
		padding: 5,
		flex: 1,
	    },
	    items: [
		{
		    xtype: 'pbsDriveInfoPanel',
		    cbind: {
			drive: '{drive}',
		    },
		},
		{
		    xtype: 'pbsDriveStatusGrid',
		    reference: 'statusgrid',
		    cbind: {
			url: '{driveStatusUrl}',
		    },
		},
	    ],
	},
    ],
});

Ext.define('PBS.TapeManagement.DriveStatusGrid', {
    extend: 'Proxmox.grid.ObjectGrid',
    alias: 'widget.pbsDriveStatusGrid',

    title: gettext('Status'),

    rows: {
	'blocksize': {
	    required: true,
	    header: gettext('Blocksize'),
	    renderer: function(value) {
		if (!value) {
		    return gettext('Dynamic');
		}
		return `${gettext('Fixed')} - ${Proxmox.Utils.format_size(value)}`;
	    },
	},
	'options': {
	    required: true,
	    header: gettext('Options'),
	    defaultValue: '',
	},
	'status': {
	    required: true,
	    header: gettext('Status'),
	},
	'density': {
	    header: gettext('Tape Density'),
	},
	'manufactured': {
	    header: gettext('Tape Manufacture Date'),
	    renderer: function(value) {
		if (value) {
		    return new Date(value*1000);
		}
		return "";
	    },
	},
	'bytes-read': {
	    header: gettext('Tape Read'),
	    renderer: Proxmox.Utils.format_size,
	},
	'bytes-written': {
	    header: gettext('Tape Written'),
	    renderer: Proxmox.Utils.format_size,
	},
    },
});

Ext.define('PBS.TapeManagement.DriveInfoPanel', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsDriveInfoPanel',

    title: gettext('Information'),

    defaults: {
	printBar: false,
	padding: 5,
    },
    bodyPadding: 15,

    viewModel: {
	data: {},
    },

    items: [
	{
	    xtype: 'pmxInfoWidget',
	    title: gettext('Name'),
	    bind: {
		data: {
		    text: '{name}',
		},
	    },
	},
	{
	    xtype: 'pmxInfoWidget',
	    title: gettext('Vendor'),
	    bind: {
		data: {
		    text: '{vendor}',
		},
	    },
	},
	{
	    xtype: 'pmxInfoWidget',
	    title: gettext('Model'),
	    bind: {
		data: {
		    text: '{model}',
		},
	    },
	},
	{
	    xtype: 'pmxInfoWidget',
	    title: gettext('Serial'),
	    bind: {
		data: {
		    text: '{serial}',
		},
	    },
	},
	{
	    xtype: 'pmxInfoWidget',
	    title: gettext('Path'),
	    bind: {
		data: {
		    text: '{path}',
		},
	    },
	},
    ],

    updateData: function(record) {
	let me = this;
	if (!record) {
	    return;
	}

	let vm = me.getViewModel();
	for (const [key, value] of Object.entries(record.data)) {
	    vm.set(key, value);
	}
    },

    initComponent: function() {
	let me = this;
	if (!me.drive) {
	    throw "no drive given";
	}

	let tapeStore = Ext.ComponentQuery.query('navigationtree')[0].tapestore;
	me.mon(tapeStore, 'load', function() {
	    let driveRecord = tapeStore.findRecord('name', me.drive, 0, false, true, true);
	    me.updateData(driveRecord);
	});
	if (!tapeStore.isLoading) {
	    let driveRecord = tapeStore.findRecord('name', me.drive, 0, false, true, true);
	    me.updateData(driveRecord);
	}

	me.callParent();
    },
});
