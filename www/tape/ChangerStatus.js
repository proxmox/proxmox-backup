Ext.define('pbs-slot-model', {
    extend: 'Ext.data.Model',
    fields: ['entry-id', 'label-text', 'is-labeled', ' model', 'name', 'vendor', 'serial', 'state', 'status', 'pool',
	{
	    name: 'is-blocked',
	    calculate: function(data) {
		return data.state !== undefined;
	    },
	},
	{
	    name: 'is-empty',
	    calculate: function(data) {
		return data['label-text'] === undefined;
	    },
	},
    ],
    idProperty: 'entry-id',
});

Ext.define('PBS.TapeManagement.FreeSlotSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsFreeSlotSelector',

    valueField: 'id',
    displayField: 'id',

    listConfig: {
	columns: [
	    {
		dataIndex: 'id',
		text: gettext('ID'),
		flex: 1,
	    },
	    {
		dataIndex: 'type',
		text: gettext('Type'),
		flex: 1,
	    },
	],
    },
});

Ext.define('PBS.TapeManagement.ChangerStatus', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsChangerStatus',

    tools: [PBS.Utils.get_help_tool("tape_backup")],

    controller: {
	xclass: 'Ext.app.ViewController',

	importTape: function(v, rI, cI, button, el, record) {
	    let me = this;
	    let view = me.getView();
	    let from = record.data['entry-id'];
	    let changer = encodeURIComponent(view.changer);
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
			xtype: 'pbsFreeSlotSelector',
			name: 'to',
			fieldLabel: gettext('To Slot'),
			store: {
			    data: me.free_slots,
			},
		    },
		],
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	slotTransfer: function(v, rI, cI, button, el, record) {
	    let me = this;
	    let view = me.getView();
	    let from = record.data['entry-id'];
	    let changer = encodeURIComponent(view.changer);
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
			xtype: 'pbsFreeSlotSelector',
			name: 'to',
			fieldLabel: gettext('To Slot'),
			store: {
			    data: me.free_slots.concat(me.free_ie_slots),
			},
		    },
		],
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	labelMedia: function(button, event, record) {
	    let me = this;
	    Ext.create('PBS.TapeManagement.LabelMediaWindow', {
		driveid: record.data.name,
		label: record.data["label-text"],
	    }).show();
	},

	catalog: function(button, event, record) {
	    let me = this;

	    let view = me.getView();
	    PBS.Utils.driveCommand(record.data.name, 'catalog', {
		waitMsgTarget: view,
		method: 'POST',
		success: function(response) {
		    Ext.create('Proxmox.window.TaskViewer', {
			upid: response.result.data,
		    }).show();
		},
	    });
	},

	'format-inserted': function(button, event, record) {
	    let me = this;

	    let view = me.getView();
	    PBS.Utils.driveCommand(record.data.name, 'format-media', {
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

	format: function(v, rI, cI, button, el, record) {
	    let me = this;
	    let view = me.getView();
	    let label = record.data['label-text'];

	    let changer = encodeURIComponent(view.changer);
	    let singleDrive = me.drives.length === 1 ? me.drives[0] : undefined;
	    Ext.create('PBS.TapeManagement.EraseWindow', {
		label,
		changer,
		singleDrive,
		listeners: {
		    destroy: function() {
			me.reload();
		    },
		},
	    }).show();
	},

	load: function(v, rI, cI, button, el, record) {
	    let me = this;
	    let view = me.getView();
	    let label = record.data['label-text'];
	    let slot = record.data['entry-id'];

	    let changer = encodeURIComponent(view.changer);
	    let singleDrive = me.drives.length === 1 ? me.drives[0] : undefined;

	    let apiCall = label !== "" ? 'load-media' : 'load-slot';
	    let params = label !== "" ? { 'label-text': label } : { 'source-slot': slot };

	    if (singleDrive !== undefined) {
		Proxmox.Utils.API2Request({
		    method: 'POST',
		    params,
		    url: `/api2/extjs/tape/drive/${singleDrive}/${apiCall}`,
		    success: function(response, opt) {
			Ext.create('Proxmox.window.TaskProgress', {
			    upid: response.result.data,
			    taskDone: function(success) {
				me.reload();
			    },
			}).show();
		    },
		    failure: function(response, opt) {
			Ext.Msg.alert(gettext('Error'), response.htmlStatus);
		    },
		});
	    } else {
		Ext.create('Proxmox.window.Edit', {
		    isCreate: true,
		    autoShow: true,
		    submitText: gettext('OK'),
		    title: gettext('Load Media into Drive'),
		    url: `/api2/extjs/tape/drive`,
		    method: 'POST',
		    submitUrl: function(url, values) {
			let drive = values.drive;
			delete values.drive;
			return `${url}/${encodeURIComponent(drive)}/${apiCall}`;
		    },
		    items: [
			label !== "" ? {
			    xtype: 'displayfield',
			    name: 'label-text',
			    value: label,
			    submitValue: true,
			    fieldLabel: gettext('Media'),
			} : {
			    xtype: 'displayfield',
			    name: 'source-slot',
			    value: slot,
			    submitValue: true,
			    fieldLabel: gettext('Source Slot'),
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
	    }
	},

	unload: async function(v, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    try {
		await Proxmox.Async.api2({
		    method: 'POST',
		    timeout: 5*60*1000,
		    url: `/api2/extjs/tape/drive/${encodeURIComponent(drive)}/unload`,
		});
	    } catch (response) {
		Ext.Msg.alert(gettext('Error'), response.result.message);
	    }
	    me.reload();
	},

	cartridgeMemory: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    PBS.Utils.driveCommand(drive, 'cartridge-memory', {
		waitMsgTarget: me.getView(),
		success: PBS.Utils.showCartridgeMemoryWindow,
	    });
	},

	cleanDrive: function(button, event, record) {
	    let me = this;
	    PBS.Utils.driveCommand(record.data.name, 'clean', {
		waitMsgTarget: me.getView(),
		method: 'PUT',
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

	volumeStatistics: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    PBS.Utils.driveCommand(drive, 'volume-statistics', {
		waitMsgTarget: me.getView(),
		success: PBS.Utils.showVolumeStatisticsWindow,
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

	status: function(view, rI, cI, button, el, record) {
	    let me = this;
	    let drive = record.data.name;
	    PBS.Utils.driveCommand(drive, 'status', {
		waitMsgTarget: me.getView(),
		success: PBS.Utils.showDriveStatusWindow,
	    });
	},

	reloadList: function() {
	    let me = this;
	    me.lookup('changerselector').getStore().load();
	},

	barcodeLabel: function() {
	    let me = this;
	    let view = me.getView();
	    let changer = view.changer;
	    if (changer === '') {
		return;
	    }

	    let singleDrive = me.drives.length === 1 ? me.drives[0] : undefined;

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
			xtype: singleDrive === undefined ? 'pbsDriveSelector' : 'displayfield',
			fieldLabel: gettext('Drive'),
			submitValue: true,
			name: 'drive',
			value: singleDrive,
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
	    let view = me.getView();
	    let changer = view.changer;
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
			labelWidth: 120,
			fieldLabel: gettext('Drive'),
			name: 'drive',
			changer: changer,
			autoSelect: true,
		    },
		    {
			xtype: 'proxmoxcheckbox',
			labelWidth: 120,
			fieldLabel: gettext('Restore Catalogs'),
			name: 'catalog',
		    },
		    {
			xtype: 'proxmoxcheckbox',
			labelWidth: 120,
			fieldLabel: gettext('Force all Tapes'),
			name: 'read-all-labels',
		    },
		],
	    }).show();
	},

	scheduleReload: function(time) {
	    let me = this;
	    if (me.reloadTimeout === undefined) {
		me.reloadTimeout = setTimeout(function() {
		    me.reload();
		}, time);
	    }
	},

	cancelReload: function() {
	    let me = this;
	    if (me.reloadTimeout !== undefined) {
		clearTimeout(me.reloadTimeout);
		me.reloadTimeout = undefined;
	    }
	},

	reload: function() {
	    let me = this;
	    me.cancelReload();
	    me.reload_full(true);
	},

	reload_no_cache: function() {
	    let me = this;
	    if (me.reloadTimeout !== undefined) {
		clearTimeout(me.reloadTimeout);
		me.reloadTimeout = undefined;
	    }
	    me.reload_full(false);
	},

	drives: [],

	updateDrives: function(drives) {
	    let me = this;
	    me.drives = drives;
	},

	free_slots: [],
	free_ie_slots: [],

	updateFreeSlots: function(free_slots, free_ie_slots) {
	    let me = this;
	    me.free_slots = free_slots;
	    me.free_ie_slots = free_ie_slots;
	},

	reload_full: async function(use_cache) {
	    let me = this;
	    let view = me.getView();
	    let changer = view.changer;
	    if (changer === '') {
		return;
	    }

	    try {
		if (!use_cache) {
		    Proxmox.Utils.setErrorMask(view, true);
		    Proxmox.Utils.setErrorMask(me.lookup('content'));
		}
		let status_fut = Proxmox.Async.api2({
		    timeout: 5*60*1000,
		    method: 'GET',
		    url: `/api2/extjs/tape/changer/${encodeURIComponent(changer)}/status`,
		    params: {
			cache: use_cache,
		    },
		});
		let drives_fut = Proxmox.Async.api2({
		    timeout: 5*60*1000,
		    url: `/api2/extjs/tape/drive?changer=${encodeURIComponent(changer)}`,
		});

		let tapes_fut = Proxmox.Async.api2({
		    timeout: 5*60*1000,
		    url: '/api2/extjs/tape/media/list',
		    method: 'GET',
		    params: {
			"update-status": false,
		    },
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

		let free_slots = [];
		let free_ie_slots = [];

		let valid_drives = [];

		for (let entry of status.result.data) {
		    let type = entry['entry-kind'];
		    let id = entry['entry-id'];

		    if (type === 'drive') {
			if (drive_entries[id] === undefined) {
			    continue;
			}

			entry = Ext.applyIf(entry, drive_entries[id]);
			valid_drives.push(drive_entries[id].name);
		    }

		    if (tapes[entry['label-text']] !== undefined) {
			entry['is-labeled'] = true;
			entry.pool = tapes[entry['label-text']].pool;
			entry.status = tapes[entry['label-text']].status;
		    } else {
			entry['is-labeled'] = false;
		    }

		    if (!entry['label-text'] && type !== 'drive') {
			if (type === 'slot') {
			    free_slots.push({
				id,
				type,
			    });
			} else {
			    free_ie_slots.push({
				id,
				type,
			    });
			}
		    }
		    data[type].push(entry);
		}

		// the stores are diffstores and are only refreshed
		// on a 'load' event, which does not trigger on 'setData'
		// so we have to fire them ourselves

		me.lookup('slots').getStore().rstore.setData(data.slot);
		me.lookup('slots').getStore().rstore.fireEvent('load', me, [], true);

		me.lookup('import_export').getStore().rstore.setData(data['import-export']);
		me.lookup('import_export').getStore().rstore.fireEvent('load', me, [], true);

		me.lookup('drives').getStore().rstore.setData(data.drive);
		me.lookup('drives').getStore().rstore.fireEvent('load', me, [], true);

		// manually fire selectionchange to update button status
		me.lookup('drives').getSelectionModel().fireEvent('selectionchange', me);

		me.updateFreeSlots(free_slots, free_ie_slots);
		me.updateDrives(valid_drives);

		if (!use_cache) {
		    Proxmox.Utils.setErrorMask(view);
		}
		Proxmox.Utils.setErrorMask(me.lookup('content'));
	    } catch (response) {
		if (!view || view.isDestroyed) {
		    return;
		}

		if (!use_cache) {
		    Proxmox.Utils.setErrorMask(view);
		}
		Proxmox.Utils.setErrorMask(me.lookup('content'), response.result.message.toString());
	    }

	    me.scheduleReload(5000);
	},

	renderLabel: function(value) {
	    if (value === undefined) {
		return '';
	    }

	    if (value === "") {
		return Ext.htmlEncode("<no-barcode>");
	    }

	    return value;
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

	control: {
	    'grid[reference=drives]': {
		cellclick: function(table, td, ci, rec, tr, ri, e) {
		    if (e.position.column.dataIndex !== 'state') {
			return;
		    }

		    let upid = rec.data.state;
		    if (!upid || !upid.startsWith("UPID")) {
			return;
		    }

		    Ext.create('Proxmox.window.TaskViewer', {
			autoShow: true,
			upid,
		    });
		},
	    },
	},

	init: function(view) {
	    let me = this;
	    if (!view.changer) {
		throw "no changer given";
	    }

	    view.title = `${gettext("Changer")}: ${view.changer}`;
	    me.reload();
	},
    },

    listeners: {
	deactivate: 'cancelReload',
	beforedestroy: 'cancelReload',
    },

    tbar: [
	{
	    text: gettext('Reload'),
	    xtype: 'proxmoxButton',
	    handler: 'reload_no_cache',
	    selModel: false,
	},
	'-',
	{
	    text: gettext('Barcode Label'),
	    xtype: 'proxmoxButton',
	    handler: 'barcodeLabel',
	    iconCls: 'fa fa-barcode',
	},
	{
	    text: gettext('Inventory'),
	    xtype: 'proxmoxButton',
	    handler: 'inventory',
	    iconCls: 'fa fa-book',
	},
    ],

    layout: 'fit',
    bodyPadding: 5,

    items: [
	{
	    xtype: 'container',
	    reference: 'content',
	    layout: {
		type: 'hbox',
		align: 'stretch',
	    },
	    items: [
		{
		    xtype: 'grid',
		    reference: 'slots',
		    title: gettext('Slots'),
		    padding: 5,
		    srollable: true,
		    flex: 1,
		    store: {
			type: 'diff',
			rstore: {
			    type: 'store',
			    model: 'pbs-slot-model',
			},
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
			    renderer: 'renderLabel',
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
				    isActionDisabled: (v, r, c, i, rec) => rec.data['is-empty'],
				},
				{
				    iconCls: 'fa fa-trash-o',
				    handler: 'format',
				    tooltip: gettext('Format'),
				    isActionDisabled: (v, r, c, i, rec) => rec.data['is-empty'],
				},
				{
				    iconCls: 'fa fa-rotate-90 fa-upload',
				    handler: 'load',
				    tooltip: gettext('Load'),
				    isActionDisabled: (v, r, c, i, rec) => rec.data['is-empty'],
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
		    layout: {
			type: 'vbox',
			align: 'stretch',
		    },
		    items: [
			{
			    xtype: 'grid',
			    reference: 'drives',
			    scrollable: true,
			    maxHeight: 350, // ~10 drives
			    title: gettext('Drives'),
			    store: {
				type: 'diff',
				rstore: {
				    type: 'store',
				    model: 'pbs-slot-model',
				},
				data: [],
			    },
			    tbar: [
				{
				    text: gettext('Label Media'),
				    xtype: 'proxmoxButton',
				    handler: 'labelMedia',
				    iconCls: 'fa fa-barcode',
				    disabled: true,
				    enableFn: (rec) => !rec.data["is-empty"],
				},
				{
				    text: gettext('Catalog'),
				    xtype: 'proxmoxButton',
				    handler: 'catalog',
				    iconCls: 'fa fa-book',
				    disabled: true,
				    enableFn: (rec) => !rec.data["is-empty"],
				},
				{
				    text: gettext('Format'),
				    xtype: 'proxmoxButton',
				    handler: 'format-inserted',
				    iconCls: 'fa fa-trash-o',
				    disabled: true,
				    enableFn: (rec) => !rec.data["is-empty"],
				    dangerous: true,
				    confirmMsg: gettext('Are you sure you want to format the inserted tape?'),
				},
				'-',
				{
				    text: gettext('Clean Drive'),
				    xtype: 'proxmoxButton',
				    handler: 'cleanDrive',
				    iconCls: 'fa fa-shower',
				    disabled: true,
				},
			    ],
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
				    renderer: 'renderLabel',
				},
				{
				    text: gettext('Inventory'),
				    dataIndex: 'is-labeled',
				    renderer: 'renderIsLabeled',
				    flex: 1.5,
				},
				{
				    text: gettext("Name"),
				    sortable: true,
				    dataIndex: 'name',
				    flex: 1,
				    renderer: Ext.htmlEncode,
				},
				{
				    text: gettext('State'),
				    dataIndex: 'state',
				    flex: 3,
				    renderer: PBS.Utils.renderDriveState,
				},
				{
				    text: gettext("Vendor"),
				    sortable: true,
				    dataIndex: 'vendor',
				    hidden: true,
				    flex: 1,
				    renderer: Ext.htmlEncode,
				},
				{
				    text: gettext("Model"),
				    sortable: true,
				    dataIndex: 'model',
				    hidden: true,
				    flex: 1,
				    renderer: Ext.htmlEncode,
				},
				{
				    text: gettext("Serial"),
				    sortable: true,
				    dataIndex: 'serial',
				    hidden: true,
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
					    isActionDisabled: (v, r, c, i, rec) => rec.data['is-empty'] || rec.data['is-blocked'],
					},
					{
					    iconCls: 'fa fa-hdd-o',
					    handler: 'cartridgeMemory',
					    tooltip: gettext('Cartridge Memory'),
					    isActionDisabled: (v, r, c, i, rec) => rec.data['is-empty'] || rec.data['is-blocked'],
					},
					{
					    iconCls: 'fa fa-line-chart',
					    handler: 'volumeStatistics',
					    tooltip: gettext('Volume Statistics'),
					    isActionDisabled: (v, r, c, i, rec) => rec.data['is-empty'] || rec.data['is-blocked'],
					},
					{
					    iconCls: 'fa fa-tag',
					    handler: 'readLabel',
					    tooltip: gettext('Read Label'),
					    isActionDisabled: (v, r, c, i, rec) => rec.data['is-empty'] || rec.data['is-blocked'],
					},
					{
					    iconCls: 'fa fa-info-circle',
					    tooltip: gettext('Status'),
					    handler: 'status',
					    isActionDisabled: (v, r, c, i, rec) => rec.data['is-blocked'],
					},
				    ],
				},
			    ],
			},
			{
			    xtype: 'grid',
			    reference: 'import_export',
			    flex: 1,
			    srollable: true,
			    store: {
				type: 'diff',
				rstore: {
				    type: 'store',
				    model: 'pbs-slot-model',
				},
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
				    renderer: 'renderLabel',
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
					    isActionDisabled: (v, r, c, i, rec) => rec.data['is-empty'],
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
