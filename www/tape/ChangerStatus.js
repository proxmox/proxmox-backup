Ext.define('PBS.TapeManagement.ChangerStatus', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsChangerStatus',

    viewModel: {
	data: {
	    changer: '',
	},

	formulas: {
	    changerSelected: (get) => get('changer') !== '',
	},
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	changerChange: function(field, value) {
	    let me = this;
	    let view = me.getView();
	    let vm = me.getViewModel();
	    vm.set('changer', value);
	    if (view.rendered) {
		me.reload();
	    }
	},

	importTape: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let vm = me.getViewModel();
	    let from = record.data['entry-id'];
	    let changer = encodeURIComponent(vm.get('changer'));
	    Ext.create('Proxmox.window.Edit', {
		title: gettext('Import'),
		isCreate: true,
		submitText: gettext('OK'),
		method: 'POST',
		url: `/api2/extjs/tape/changer/${changer}/transfer`,
		items: [
		    {
			xtype: 'displayfield',
			name: 'from',
			value: from,
			submitValue: true,
			fieldLabel: gettext('From Slot'),
		    },
		    {
			xtype: 'proxmoxintegerfield',
			name: 'to',
			fieldLabel: gettext('To Slot'),
		    },
		],
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	slotTransfer: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let vm = me.getViewModel();
	    let from = record.data['entry-id'];
	    let changer = encodeURIComponent(vm.get('changer'));
	    Ext.create('Proxmox.window.Edit', {
		title: gettext('Transfer'),
		isCreate: true,
		submitText: gettext('OK'),
		method: 'POST',
		url: `/api2/extjs/tape/changer/${changer}/transfer`,
		items: [
		    {
			xtype: 'displayfield',
			name: 'from',
			value: from,
			submitValue: true,
			fieldLabel: gettext('From Slot'),
		    },
		    {
			xtype: 'proxmoxintegerfield',
			name: 'to',
			fieldLabel: gettext('To Slot'),
		    },
		],
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	erase: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let vm = me.getViewModel();
	    let label = record.data['label-text'];

	    let changer = vm.get('changer');
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

	load: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let vm = me.getViewModel();
	    let label = record.data['label-text'];

	    let changer = vm.get('changer');

	    Ext.create('Proxmox.window.Edit', {
		isCreate: true,
		autoShow: true,
		submitText: gettext('OK'),
		title: gettext('Load Media into Drive'),
		url: `/api2/extjs/tape/drive`,
		showProgress: true,
		method: 'POST',
		submitUrl: function(url, values) {
		    let drive = values.drive;
		    delete values.drive;
		    return `${url}/${encodeURIComponent(drive)}/load-media`;
		},
		items: [
		    {
			xtype: 'displayfield',
			name: 'label-text',
			value: label,
			submitValue: true,
			fieldLabel: gettext('Media'),
		    },
		    {
			xtype: 'pbsDriveSelector',
			fieldLabel: gettext('Drive'),
			changer: changer,
			name: 'drive',
		    },
		],
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    });
	},

	unload: async function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    try {
		let response = await PBS.Async.api2({
		    method: 'POST',
		    timeout: 5*60*1000,
		    url: `/api2/extjs/tape/drive/${encodeURIComponent(drive)}/unload`,
		});

		Ext.create('Proxmox.window.TaskProgress', {
		    autoShow: true,
		    upid: response.result.data,
		    taskDone: () => me.reload(),
		});
	    } catch (error) {
		Ext.Msg.alert(gettext('Error'), error);
		me.reload();
	    }
	},

	driveCommand: function(driveid, command, callback, params, method) {
	    let me = this;
	    let view = me.getView();
	    params = params || {};
	    method = method || 'GET';
	    Proxmox.Utils.API2Request({
		url: `/api2/extjs/tape/drive/${driveid}/${command}`,
		timeout: 5*60*1000,
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

	cleanDrive: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    me.driveCommand(drive, 'clean', function(response) {
		Ext.create('Proxmox.window.TaskProgress', {
		    upid: response.result.data,
		    taskDone: function() {
			me.reload();
		    },
		}).show();
	    }, {}, 'PUT');
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
		    title: gettext('Status'),
		    message: txt,
		    icon: undefined,
		});
	    });
	},

	reloadList: function() {
	    let me = this;
	    me.lookup('changerselector').getStore().load();
	},

	barcodeLabel: function() {
	    let me = this;
	    let vm = me.getViewModel();
	    let changer = vm.get('changer');
	    if (changer === '') {
		return;
	    }

	    Ext.create('Proxmox.window.Edit', {
		title: gettext('Barcode Label'),
		showTaskViewer: true,
		method: 'POST',
		url: '/api2/extjs/tape/drive',
		submitUrl: function(url, values) {
		    let drive = values.drive;
		    delete values.drive;
		    return `${url}/${encodeURIComponent(drive)}/barcode-label-media`;
		},

		items: [
		    {
			xtype: 'pbsDriveSelector',
			fieldLabel: gettext('Drive'),
			name: 'drive',
			changer: changer,
		    },
		    {
			xtype: 'pbsMediaPoolSelector',
			fieldLabel: gettext('Pool'),
			name: 'pool',
			skipEmptyText: true,
			allowBlank: true,
		    },
		],
	    }).show();
	},

	inventory: function() {
	    let me = this;
	    let vm = me.getViewModel();
	    let changer = vm.get('changer');
	    if (changer === '') {
		return;
	    }

	    Ext.create('Proxmox.window.Edit', {
		title: gettext('Inventory'),
		showTaskViewer: true,
		method: 'PUT',
		url: '/api2/extjs/tape/drive',
		submitUrl: function(url, values) {
		    let drive = values.drive;
		    delete values.drive;
		    return `${url}/${encodeURIComponent(drive)}/inventory`;
		},

		items: [
		    {
			xtype: 'pbsDriveSelector',
			fieldLabel: gettext('Drive'),
			name: 'drive',
			changer: changer,
		    },
		],
	    }).show();
	},

	reload: async function() {
	    let me = this;
	    let view = me.getView();
	    let vm = me.getViewModel();
	    let changer = vm.get('changer');
	    if (changer === '') {
		return;
	    }

	    try {
		Proxmox.Utils.setErrorMask(view, true);
		Proxmox.Utils.setErrorMask(me.lookup('content'));
		let status_fut = PBS.Async.api2({
		    timeout: 5*60*1000,
		    url: `/api2/extjs/tape/changer/${encodeURIComponent(changer)}/status`,
		});
		let drives_fut = PBS.Async.api2({
		    timeout: 5*60*1000,
		    url: `/api2/extjs/tape/drive?changer=${encodeURIComponent(changer)}`,
		});

		let tapes_fut = PBS.Async.api2({
		    timeout: 5*60*1000,
		    url: '/api2/extjs/tape/media/list',
		    method: 'GET',
		    params: {
			"update-status": false,
		    }
		});

		let [status, drives, tapes_list] = await Promise.all([status_fut, drives_fut, tapes_fut]);

		let data = {
		    slot: [],
		    'import-export': [],
		    drive: [],
		};

		let tapes = {};

		for (const tape of tapes_list.result.data) {
		    tapes[tape['label-text']] = {
			labeled: true,
			pool: tape.pool,
			status: tape.expired ? 'expired' : tape.status,
		    };
		}

		let drive_entries = {};

		for (const entry of drives.result.data) {
		    drive_entries[entry['changer-drivenum'] || 0] = entry;
		}

		for (let entry of status.result.data) {
		    let type = entry['entry-kind'];

		    if (type === 'drive' && drive_entries[entry['entry-id']] !== undefined) {
			entry = Ext.applyIf(entry, drive_entries[entry['entry-id']]);
		    }

		    if (tapes[entry['label-text']] !== undefined) {
			entry['is-labeled'] = true;
			entry.pool = tapes[entry['label-text']].pool;
			entry.status = tapes[entry['label-text']].status;
		    } else {
			entry['is-labeled'] = false;
		    }

		    data[type].push(entry);
		}


		me.lookup('slots').getStore().setData(data.slot);
		me.lookup('import_export').getStore().setData(data['import-export']);
		me.lookup('drives').getStore().setData(data.drive);

		Proxmox.Utils.setErrorMask(view);
	    } catch (err) {
		Proxmox.Utils.setErrorMask(view);
		Proxmox.Utils.setErrorMask(me.lookup('content'), err);
	    }
	},

	renderIsLabeled: function(value, mD, record) {
	    if (!record.data['label-text']) {
		return "";
	    }

	    if (record.data['label-text'].startsWith("CLN")) {
		return "";
	    }

	    if (!value) {
		return gettext('Not Labeled');
	    }

	    let status = record.data.status;
	    if (record.data.pool) {
		return `${status} (${record.data.pool})`;
	    }
	    return status;
	},
    },

    listeners: {
	activate: 'reload',
    },

    tbar: [
	{
	    fieldLabel: gettext('Changer'),
	    xtype: 'pbsChangerSelector',
	    reference: 'changerselector',
	    autoSelect: true,
	    listeners: {
		change: 'changerChange',
	    },
	},
	'-',
	{
	    text: gettext('Reload'),
	    xtype: 'proxmoxButton',
	    handler: 'reload',
	    selModel: false,
	},
	'-',
	{
	    text: gettext('Barcode Label'),
	    xtype: 'proxmoxButton',
	    handler: 'barcodeLabel',
	    iconCls: 'fa fa-barcode',
	    bind: {
		disabled: '{!changerSelected}',
	    },
	},
	{
	    text: gettext('Inventory'),
	    xtype: 'proxmoxButton',
	    handler: 'inventory',
	    iconCls: 'fa fa-book',
	    bind: {
		disabled: '{!changerSelected}',
	    },
	},
    ],

    layout: 'auto',
    bodyPadding: 5,
    scrollable: true,

    items: [
	{
	    xtype: 'container',
	    reference: 'content',
	    layout: {
		type: 'hbox',
		aling: 'stretch',
	    },
	    items: [
		{
		    xtype: 'grid',
		    reference: 'slots',
		    title: gettext('Slots'),
		    padding: 5,
		    flex: 1,
		    store: {
			data: [],
		    },
		    columns: [
			{
			    text: gettext('ID'),
			    dataIndex: 'entry-id',
			    width: 50,
			},
			{
			    text: gettext("Content"),
			    dataIndex: 'label-text',
			    flex: 1,
			    renderer: (value) => value || '',
			},
			{
			    text: gettext('Inventory'),
			    dataIndex: 'is-labeled',
			    renderer: 'renderIsLabeled',
			    flex: 1,
			},
			{
			    text: gettext('Actions'),
			    xtype: 'actioncolumn',
			    width: 100,
			    items: [
				{
				    iconCls: 'fa fa-rotate-90 fa-exchange',
				    handler: 'slotTransfer',
				    tooltip: gettext('Transfer'),
				    isDisabled: (v, r, c, i, rec) => !rec.data['label-text'],
				},
				{
				    iconCls: 'fa fa-trash-o',
				    handler: 'erase',
				    tooltip: gettext('Erase'),
				    isDisabled: (v, r, c, i, rec) => !rec.data['label-text'],
				},
				{
				    iconCls: 'fa fa-rotate-90 fa-upload',
				    handler: 'load',
				    tooltip: gettext('Load'),
				    isDisabled: (v, r, c, i, rec) => !rec.data['label-text'],
				},
			    ],
			},
		    ],
		},
		{
		    xtype: 'container',
		    flex: 2,
		    defaults: {
			padding: 5,
		    },
		    items: [
			{
			    xtype: 'grid',
			    reference: 'drives',
			    title: gettext('Drives'),
			    store: {
				fields: ['entry-id', 'label-text', 'model', 'name', 'vendor', 'serial'],
				data: [],
			    },
			    columns: [
				{
				    text: gettext('ID'),
				    dataIndex: 'entry-id',
				    hidden: true,
				    width: 50,
				},
				{
				    text: gettext("Content"),
				    dataIndex: 'label-text',
				    flex: 1,
				    renderer: (value) => value || '',
				},
				{
				    text: gettext('Inventory'),
				    dataIndex: 'is-labeled',
				    renderer: 'renderIsLabeled',
				    flex: 1,
				},
				{
				    text: gettext("Name"),
				    sortable: true,
				    dataIndex: 'name',
				    flex: 1,
				    renderer: Ext.htmlEncode,
				},
				{
				    text: gettext("Vendor"),
				    sortable: true,
				    dataIndex: 'vendor',
				    flex: 1,
				    renderer: Ext.htmlEncode,
				},
				{
				    text: gettext("Model"),
				    sortable: true,
				    dataIndex: 'model',
				    flex: 1,
				    renderer: Ext.htmlEncode,
				},
				{
				    text: gettext("Serial"),
				    sortable: true,
				    dataIndex: 'serial',
				    flex: 1,
				    renderer: Ext.htmlEncode,
				},
				{
				    xtype: 'actioncolumn',
				    text: gettext('Actions'),
				    width: 140,
				    items: [
					{
					    iconCls: 'fa fa-rotate-270 fa-upload',
					    handler: 'unload',
					    tooltip: gettext('Unload'),
					    isDisabled: (v, r, c, i, rec) => !rec.data['label-text'],
					},
					{
					    iconCls: 'fa fa-hdd-o',
					    handler: 'cartridgeMemory',
					    tooltip: gettext('Cartridge Memory'),
					    isDisabled: (v, r, c, i, rec) => !rec.data['label-text'],
					},
					{
					    iconCls: 'fa fa-line-chart',
					    handler: 'volumeStatistics',
					    tooltip: gettext('Volume Statistics'),
					    isDisabled: (v, r, c, i, rec) => !rec.data['label-text'],
					},
					{
					    iconCls: 'fa fa-tag',
					    handler: 'readLabel',
					    tooltip: gettext('Read Label'),
					    isDisabled: (v, r, c, i, rec) => !rec.data['label-text'],
					},
					{
					    iconCls: 'fa fa-info-circle',
					    tooltip: gettext('Status'),
					    handler: 'status',
					},
					{
					    iconCls: 'fa fa-shower',
					    tooltip: gettext('Clean Drive'),
					    handler: 'cleanDrive',
					},
				    ],
				},
			    ],
			},
			{
			    xtype: 'grid',
			    reference: 'import_export',
			    store: {
				data: [],
			    },
			    title: gettext('Import-Export Slots'),
			    columns: [
				{
				    text: gettext('ID'),
				    dataIndex: 'entry-id',
				    width: 50,
				},
				{
				    text: gettext("Content"),
				    dataIndex: 'label-text',
				    renderer: (value) => value || '',
				    flex: 1,
				},
				{
				    text: gettext('Inventory'),
				    dataIndex: 'is-labeled',
				    renderer: 'renderIsLabeled',
				    flex: 1,
				},
				{
				    text: gettext('Actions'),
				    xtype: 'actioncolumn',
				    items: [
					{
					    iconCls: 'fa fa-rotate-270 fa-upload',
					    handler: 'importTape',
					    tooltip: gettext('Import'),
					    isDisabled: (v, r, c, i, rec) => !rec.data['label-text'],
					},
				    ],
				    width: 80,
				},
			    ],
			},
		    ],
		},
	    ],
	},
    ],
});
