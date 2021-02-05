Ext.define('pbs-model-drives', {
    extend: 'Ext.data.Model',
    fields: ['path', 'model', 'name', 'serial', 'vendor', 'changer', 'changer-slot'],
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
	    me.driveCommand(drive, 'status', function(response) {
		let lines = [];
		for (const [key, val] of Object.entries(response.result.data)) {
		    lines.push(`${key}: ${val}`);
		}

		let txt = lines.join('<br>');

		Ext.Msg.show({
		    title: gettext('Label Information'),
		    message: txt,
		    icon: undefined,
		});
	    });
	},

	catalog: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    me.driveCommand(drive, 'catalog', function(response) {
		Ext.create('Proxmox.window.TaskViewer', {
		    upid: response.result.data,
		}).show();
	    }, {}, 'POST');
	},

	readLabel: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    me.driveCommand(drive, 'read-label', function(response) {
		let lines = [];
		for (const [key, val] of Object.entries(response.result.data)) {
		    lines.push(`${key}: ${val}`);
		}

		let txt = lines.join('<br>');

		Ext.Msg.show({
		    title: gettext('Label Information'),
		    message: txt,
		    icon: undefined,
		});
	    });
	},

	volumeStatistics: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    me.driveCommand(drive, 'volume-statistics', function(response) {
		Ext.create('Ext.window.Window', {
		    title: gettext('Volume Statistics'),
		    modal: true,
		    width: 600,
		    height: 450,
		    layout: 'fit',
		    scrollable: true,
		    items: [
			{
			    xtype: 'grid',
			    store: {
				data: response.result.data,
			    },
			    columns: [
				{
				    text: gettext('ID'),
				    dataIndex: 'id',
				    width: 60,
				},
				{
				    text: gettext('Name'),
				    dataIndex: 'name',
				    flex: 2,
				},
				{
				    text: gettext('Value'),
				    dataIndex: 'value',
				    flex: 1,
				},
			    ],
			},
		    ],
		}).show();
	    });
	},

	cartridgeMemory: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    me.driveCommand(drive, 'cartridge-memory', function(response) {
		Ext.create('Ext.window.Window', {
		    title: gettext('Cartridge Memory'),
		    modal: true,
		    width: 600,
		    height: 450,
		    layout: 'fit',
		    scrollable: true,
		    items: [
			{
			    xtype: 'grid',
			    store: {
				data: response.result.data,
			    },
			    columns: [
				{
				    text: gettext('ID'),
				    dataIndex: 'id',
				    width: 60,
				},
				{
				    text: gettext('Name'),
				    dataIndex: 'name',
				    flex: 2,
				},
				{
				    text: gettext('Value'),
				    dataIndex: 'value',
				    flex: 1,
				},
			    ],
			},
		    ],
		}).show();
	    });
	},

	driveCommand: function(driveid, command, callback, params, method) {
	    let me = this;
	    let view = me.getView();
	    params = params || {};
	    method = method || 'GET';
	    Proxmox.Utils.API2Request({
		url: `/api2/extjs/tape/drive/${driveid}/${command}`,
		method,
		waitMsgTarget: view,
		params,
		success: function(response) {
		    callback(response);
		},
		failure: function(response) {
		    Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		},
	    });
	},

	labelMedia: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let driveid = record.data.name;

	    Ext.create('PBS.TapeManagement.LabelMediaWindow', {
		driveid,
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
    },

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
	    text: gettext('Changer'),
	    flex: 1,
	    dataIndex: 'changer',
	    renderer: function(value, mD, record) {
		if (!value) {
		    return "";
		}
		let drive_num = record.data['changer-drivenum'] || 0;
		let drive_text = gettext("Drive {0}");
		return `${value} (${Ext.String.format(drive_text, drive_num)})`;
	    },
	    sorter: function(a, b) {
		let ch_a = a.data.changer || "";
		let ch_b = b.data.changer || "";
		let num_a = a.data['changer-drivenum'] || 0;
		let num_b = b.data['changer-drivenum'] || 0;
		return ch_a > ch_b ? -1 : ch_a < ch_b ? 1 : num_b - num_a;
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
		},
		{
		    iconCls: 'fa fa-line-chart',
		    handler: 'volumeStatistics',
		},
		{
		    iconCls: 'fa fa-tag',
		    handler: 'readLabel',
		},
		{
		    iconCls: 'fa fa-book',
		    handler: 'catalog',
		    tooltip: gettext('Catalog'),
		},
		{
		    iconCls: 'fa fa-info-circle',
		    handler: 'status',
		},
		{
		    iconCls: 'fa fa-pencil-square-o',
		    handler: 'labelMedia',
		},
	    ],
	},
    ],
});

