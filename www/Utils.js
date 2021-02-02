Ext.ns('PBS');

console.log("Starting Backup Server GUI");

Ext.define('PBS.Utils', {
    singleton: true,

    updateLoginData: function(data) {
	Proxmox.Utils.setAuthData(data);
    },

    dataStorePrefix: 'DataStore-',

    cryptmap: [
	'none',
	'mixed',
	'sign-only',
	'encrypt',
    ],

    cryptText: [
	Proxmox.Utils.noText,
	gettext('Mixed'),
	gettext('Signed'),
	gettext('Encrypted'),
    ],

    cryptIconCls: [
	'',
	'',
	'lock faded',
	'lock good',
    ],

    calculateCryptMode: function(data) {
	let mixed = data.mixed;
	let encrypted = data.encrypt;
	let signed = data['sign-only'];
	let files = data.count;
	if (mixed > 0) {
	    return PBS.Utils.cryptmap.indexOf('mixed');
	} else if (files === encrypted && encrypted > 0) {
	    return PBS.Utils.cryptmap.indexOf('encrypt');
	} else if (files === signed && signed > 0) {
	    return PBS.Utils.cryptmap.indexOf('sign-only');
	} else if ((signed+encrypted) === 0) {
	    return PBS.Utils.cryptmap.indexOf('none');
	} else {
	    return PBS.Utils.cryptmap.indexOf('mixed');
	}
    },

    noSubKeyHtml: 'You do not have a valid subscription for this server. Please visit <a target="_blank" href="https://www.proxmox.com/proxmox-backup-server/pricing">www.proxmox.com</a> to get a list of available options.',

    getDataStoreFromPath: function(path) {
	return path.slice(PBS.Utils.dataStorePrefix.length);
    },

    isDataStorePath: function(path) {
	return path.indexOf(PBS.Utils.dataStorePrefix) === 0;
    },

    parsePropertyString: function(value, defaultKey) {
	var res = {},
	    error;

	if (typeof value !== 'string' || value === '') {
	    return res;
	}

	Ext.Array.each(value.split(','), function(p) {
	    var kv = p.split('=', 2);
	    if (Ext.isDefined(kv[1])) {
		res[kv[0]] = kv[1];
	    } else if (Ext.isDefined(defaultKey)) {
		if (Ext.isDefined(res[defaultKey])) {
		    error = 'defaultKey may be only defined once in propertyString';
		    return false; // break
		}
		res[defaultKey] = kv[0];
	    } else {
		error = 'invalid propertyString, not a key=value pair and no defaultKey defined';
		return false; // break
	    }
	    return true;
	});

	if (error !== undefined) {
	    console.error(error);
	    return null;
	}

	return res;
    },

    printPropertyString: function(data, defaultKey) {
	var stringparts = [],
	    gotDefaultKeyVal = false,
	    defaultKeyVal;

	Ext.Object.each(data, function(key, value) {
	    if (defaultKey !== undefined && key === defaultKey) {
		gotDefaultKeyVal = true;
		defaultKeyVal = value;
	    } else if (value !== '' && value !== undefined) {
		stringparts.push(key + '=' + value);
	    }
	});

	stringparts = stringparts.sort();
	if (gotDefaultKeyVal) {
	    stringparts.unshift(defaultKeyVal);
	}

	return stringparts.join(',');
    },

    // helper for deleting field which are set to there default values
    delete_if_default: function(values, fieldname, default_val, create) {
	if (values[fieldname] === '' || values[fieldname] === default_val) {
	    if (!create) {
		if (values.delete) {
		    if (Ext.isArray(values.delete)) {
			values.delete.push(fieldname);
		    } else {
			values.delete += ',' + fieldname;
		    }
		} else {
		    values.delete = [fieldname];
		}
	    }

	    delete values[fieldname];
	}
    },


    render_datetime_utc: function(datetime) {
	let pad = (number) => number < 10 ? '0' + number : number;
	return datetime.getUTCFullYear() +
	    '-' + pad(datetime.getUTCMonth() + 1) +
	    '-' + pad(datetime.getUTCDate()) +
	    'T' + pad(datetime.getUTCHours()) +
	    ':' + pad(datetime.getUTCMinutes()) +
	    ':' + pad(datetime.getUTCSeconds()) +
	    'Z';
    },

    render_datastore_worker_id: function(id, what) {
	const res = id.match(/^(\S+?):(\S+?)\/(\S+?)(\/(.+))?$/);
	if (res) {
	    let datastore = res[1], backupGroup = `${res[2]}/${res[3]}`;
	    if (res[4] !== undefined) {
		let datetime = Ext.Date.parse(parseInt(res[5], 16), 'U');
		let utctime = PBS.Utils.render_datetime_utc(datetime);
		return `Datastore ${datastore} ${what} ${backupGroup}/${utctime}`;
	    } else {
		return `Datastore ${datastore} ${what} ${backupGroup}`;
	    }
	}
	return `Datastore ${what} ${id}`;
    },

    // mimics Display trait in backend
    renderKeyID: function(fingerprint) {
	return fingerprint.substring(0, 23);
    },

    parse_datastore_worker_id: function(type, id) {
	let result;
	let res;
	if (type.startsWith('verif')) {
	    res = PBS.Utils.VERIFICATION_JOB_ID_RE.exec(id);
	    if (res) {
		result = res[1];
	    }
	} else if (type.startsWith('sync')) {
	    res = PBS.Utils.SYNC_JOB_ID_RE.exec(id);
	    if (res) {
		result = res[3];
	    }
	} else if (type === 'backup') {
	    res = PBS.Utils.BACKUP_JOB_ID_RE.exec(id);
	    if (res) {
		result = res[1];
	    }
	} else if (type === 'garbage_collection') {
	    return id;
	} else if (type === 'prune') {
	    return id;
	}


	return result;
    },

    extractTokenUser: function(tokenid) {
	return tokenid.match(/^(.+)!([^!]+)$/)[1];
    },

    extractTokenName: function(tokenid) {
	return tokenid.match(/^(.+)!([^!]+)$/)[2];
    },

    render_estimate: function(value) {
	if (!value) {
	    return gettext('Not enough data');
	}

	let now = new Date();
	let estimate = new Date(value*1000);

	let timespan = (estimate - now)/1000;

	if (Number(estimate) <= Number(now) || isNaN(timespan)) {
	    return gettext('Never');
	}

	let duration = Proxmox.Utils.format_duration_human(timespan);
	return Ext.String.format(gettext("in {0}"), duration);
    },

    render_size_usage: function(val, max) {
	if (max === 0) {
	    return gettext('N/A');
	}
	return (val*100/max).toFixed(2) + '% (' +
	    Ext.String.format(gettext('{0} of {1}'),
	    Proxmox.Utils.format_size(val), Proxmox.Utils.format_size(max)) + ')';
    },

    get_help_tool: function(blockid) {
	let info = Proxmox.Utils.get_help_info(blockid);
	if (info === undefined) {
	    info = Proxmox.Utils.get_help_info('pbs_documentation_index');
	}
	if (info === undefined) {
	    throw "get_help_info failed"; // should not happen
	}

	let docsURI = window.location.origin + info.link;
	let title = info.title;
	if (info.subtitle) {
	    title += ' - ' + info.subtitle;
	}
	return {
	    type: 'help',
	    tooltip: title,
	    handler: function() {
		window.open(docsURI);
	    },
        };
    },

    calculate_dedup_factor: function(gcstatus) {
	let dedup = 1.0;
	if (gcstatus['disk-bytes'] > 0) {
	    dedup = (gcstatus['index-data-bytes'] || 0)/gcstatus['disk-bytes'];
	}
	return dedup;
    },

    parse_snapshot_id: function(snapshot) {
	if (!snapshot) {
	    return [undefined, undefined, undefined];
	}
	let [_match, type, group, id] = /^([^/]+)\/([^/]+)\/(.+)$/.exec(snapshot);

	return [type, group, id];
    },

    get_type_icon_cls: function(btype) {
	var cls = '';
	if (btype.startsWith('vm')) {
	    cls = 'fa-desktop';
	} else if (btype.startsWith('ct')) {
	    cls = 'fa-cube';
	} else if (btype.startsWith('host')) {
	    cls = 'fa-building';
	}
	return cls;
    },

    constructor: function() {
	var me = this;

	let PROXMOX_SAFE_ID_REGEX = "([A-Za-z0-9_][A-Za-z0-9._-]*)";
	// only anchored at beginning
	// only parses datastore for now
	me.VERIFICATION_JOB_ID_RE = new RegExp("^" + PROXMOX_SAFE_ID_REGEX + ':?');
	me.SYNC_JOB_ID_RE = new RegExp("^" + PROXMOX_SAFE_ID_REGEX + ':' +
	    PROXMOX_SAFE_ID_REGEX + ':' + PROXMOX_SAFE_ID_REGEX + ':');
	me.BACKUP_JOB_ID_RE = new RegExp("^" + PROXMOX_SAFE_ID_REGEX + ':');

	// do whatever you want here
	Proxmox.Utils.override_task_descriptions({
	    backup: (type, id) => PBS.Utils.render_datastore_worker_id(id, gettext('Backup')),
	    "tape-backup": ['Datastore', gettext('Tape Backup')],
	    "tape-restore": ['Datastore', gettext('Tape Restore')],
	    "barcode-label-media": [gettext('Drive'), gettext('Barcode label media')],
	    dircreate: [gettext('Directory Storage'), gettext('Create')],
	    dirremove: [gettext('Directory'), gettext('Remove')],
	    "eject-media": [gettext('Drive'), gettext('Eject media')],
	    "erase-media": [gettext('Drive'), gettext('Erase media')],
	    garbage_collection: ['Datastore', gettext('Garbage collect')],
	    "inventory-update": [gettext('Drive'), gettext('Inventory update')],
	    "label-media": [gettext('Drive'), gettext('Label media')],
	    "catalog-media": [gettext('Drive'), gettext('Catalog media')],
	    logrotate: [null, gettext('Log Rotation')],
	    prune: (type, id) => PBS.Utils.render_datastore_worker_id(id, gettext('Prune')),
	    reader: (type, id) => PBS.Utils.render_datastore_worker_id(id, gettext('Read objects')),
	    "rewind-media": [gettext('Drive'), gettext('Rewind media')],
	    sync: ['Datastore', gettext('Remote Sync')],
	    syncjob: [gettext('Sync Job'), gettext('Remote Sync')],
	    verify: ['Datastore', gettext('Verification')],
	    verify_group: ['Group', gettext('Verification')],
	    verify_snapshot: ['Snapshot', gettext('Verification')],
	    verificationjob: [gettext('Verify Job'), gettext('Scheduled Verification')],
	    zfscreate: [gettext('ZFS Storage'), gettext('Create')],
	});
    },

    // Convert an ArrayBuffer to a base64url encoded string.
    // A `null` value will be preserved for convenience.
    bytes_to_base64url: function(bytes) {
	if (bytes === null) {
	    return null;
	}

	return btoa(Array
	    .from(new Uint8Array(bytes))
	    .map(val => String.fromCharCode(val))
	    .join(''),
	)
	.replace(/\+/g, '-')
	.replace(/\//g, '_')
	.replace(/[=]/g, '');
    },

    // Convert an a base64url string to an ArrayBuffer.
    // A `null` value will be preserved for convenience.
    base64url_to_bytes: function(b64u) {
	if (b64u === null) {
	    return null;
	}

	return new Uint8Array(
	    atob(b64u
		.replace(/-/g, '+')
		.replace(/_/g, '/'),
	    )
	    .split('')
	    .map(val => val.charCodeAt(0)),
	);
    },
});

Ext.define('PBS.Async', {
    singleton: true,

    // Returns a Promise resolving to the result of an `API2Request`.
    api2: function(reqOpts) {
	return new Promise((resolve, reject) => {
	    delete reqOpts.callback; // not allowed in this api
	    reqOpts.success = response => resolve(response);
	    reqOpts.failure = response => {
		if (response.result && response.result.message) {
		    reject(response.result.message);
		} else {
		    reject("api call failed");
		}
	    };
	    Proxmox.Utils.API2Request(reqOpts);
	});
    },

    // Delay for a number of milliseconds.
    sleep: function(millis) {
	return new Promise((resolve, _reject) => setTimeout(resolve, millis));
    },
});
