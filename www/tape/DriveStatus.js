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

    controller: {
	xclass: 'Ext.app.ViewController',

	reload: function() {
	    let me = this;
	    me.lookup('statusgrid').rstore.load();
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
