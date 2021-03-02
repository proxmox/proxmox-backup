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
	    busy: true,
	    loaded: false,
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

	onStateLoad: function(store) {
	    let me = this;
	    let view = me.getView();
	    let vm = me.getViewModel();
	    let driveRecord = store.findRecord('name', view.drive, 0, false, true, true);
	    let busy = !!driveRecord.data.state;
	    vm.set('busy', busy);
	    let statusgrid = me.lookup('statusgrid');
	    if (!vm.get('loaded')) {
		if (busy) {
		    // have to use a timeout so that the component can be rendered first
		    // otherwise the 'mask' call errors out
		    setTimeout(function() {
			statusgrid.mask(gettext('Drive is busy'));
		    }, 10);
		} else {
		    // have to use a timeout so that the component can be rendered first
		    // otherwise the 'mask' call errors out
		    setTimeout(function() {
			statusgrid.unmask();
		    }, 10);
		    me.reload();
		    vm.set('loaded', true);
		}
	    }
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
	    let tapeStore = Ext.ComponentQuery.query('navigationtree')[0].tapestore;
	    me.mon(tapeStore, 'load', 'onStateLoad');
	    if (tapeStore.isLoaded()) {
		me.onStateLoad(tapeStore);
	    }
	},
    },

    tbar: [
	{
	    xtype: 'proxmoxButton',
	    handler: 'reload',
	    text: gettext('Reload'),
	    disabled: true,
	    bind: {
		disabled: '{busy}',
	    },
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
	'medium-passes': {
	    header: gettext('Tape Passes'),
	},
	'medium-wearout': {
	    header: gettext('Tape Wearout'),
	    renderer: function(value) {
		if (value !== undefined) {
		    return (value*100).toFixed(2) + "%";
		}
		return value;
	    },
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
	data: {
	    drive: {},
	},

	formulas: {
	    driveState: function(get) {
		let drive = get('drive');
		return PBS.Utils.renderDriveState(drive.state, {});
	    },
	},
    },

    items: [
	{
	    xtype: 'pmxInfoWidget',
	    title: gettext('Name'),
	    bind: {
		data: {
		    text: '{drive.name}',
		},
	    },
	},
	{
	    xtype: 'pmxInfoWidget',
	    title: gettext('Vendor'),
	    bind: {
		data: {
		    text: '{drive.vendor}',
		},
	    },
	},
	{
	    xtype: 'pmxInfoWidget',
	    title: gettext('Model'),
	    bind: {
		data: {
		    text: '{drive.model}',
		},
	    },
	},
	{
	    xtype: 'pmxInfoWidget',
	    title: gettext('Serial'),
	    bind: {
		data: {
		    text: '{drive.serial}',
		},
	    },
	},
	{
	    xtype: 'pmxInfoWidget',
	    title: gettext('Path'),
	    bind: {
		data: {
		    text: '{drive.path}',
		},
	    },
	},
	{
	    xtype: 'pmxInfoWidget',
	    title: gettext('State'),
	    bind: {
		data: {
		    text: '{driveState}',
		},
	    },
	},
    ],

    updateData: function(store) {
	let me = this;
	if (!store) {
	    return;
	}
	let record = store.findRecord('name', me.drive, 0, false, true, true);
	if (!record) {
	    return;
	}

	let vm = me.getViewModel();
	vm.set('drive', record.data);
	vm.notify();
    },

    initComponent: function() {
	let me = this;
	if (!me.drive) {
	    throw "no drive given";
	}

	let tapeStore = Ext.ComponentQuery.query('navigationtree')[0].tapestore;
	me.mon(tapeStore, 'load', me.updateData, me);
	if (tapeStore.isLoaded()) {
	    me.updateData(tapeStore);
	}

	me.callParent();
    },
});
